//! Lua scripts: small programs that react to events and run Rosorah's commands. Each runs in its
//! own sandbox (no files, system or network), with a memory cap and a time budget per call.
use mlua::{Function, HookTriggers, Lua, LuaOptions, RegistryKey, StdLib, Table, Value as LuaValue, VmState};
use serde_json::Value;
use std::{
    cell::{Cell, RefCell},
    collections::VecDeque,
    path::{Path, PathBuf},
    rc::Rc,
    time::{Duration, Instant},
};

/// A command a script asked for; the studio carries it out after the script returns.
pub struct Request {
    pub script: String,
    pub command: String,
    pub data: Value,
}
struct Timer {
    every: Duration,
    next: Instant,
    function: RegistryKey,
}
struct Script {
    name: String,
    lua: Lua,
    handlers: Rc<RefCell<Vec<(String, RegistryKey)>>>,
    timers: Rc<RefCell<Vec<Timer>>>,
    requests: Rc<RefCell<Vec<(String, Value)>>>,
    budget: Rc<Cell<u32>>,
    log: Rc<RefCell<VecDeque<String>>>,
}
#[derive(Default)]
pub struct Scripts {
    running: Vec<Script>,
    /// Problems loading or running scripts, by script name (shown on the Scripts page).
    pub errors: std::collections::BTreeMap<String, String>,
}
/// Where scripts live.
pub fn folder() -> PathBuf {
    auroracast_core::platform::config_root().join("rosorah-studio/scripts")
}
/// The .lua files in the scripts folder, by name.
pub fn available() -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(folder())
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().to_str().filter(|n| n.ends_with(".lua")).map(str::to_owned))
        .collect();
    names.sort();
    names
}
/// An example to start from, written when the folder is first made.
pub const EXAMPLE: &str = r#"-- Example Rosorah script. Switch it on in Settings > Scripts.
-- rosorah.on(event, function(e) ... end)   events: follow, subscription, gift, donation, raid,
--   bits, redemption, chat, scene, preview, live, recording, replay, virtual_camera, mute
-- rosorah.every(seconds, function() ... end)
-- rosorah.run(command, {…})   the same commands as remote control (see Settings > Remote control)
-- rosorah.status()            what's live now; rosorah.log(text) writes to the Scripts page

rosorah.on("follow", function(e)
  rosorah.run("alert", { text = "Welcome, " .. e.author .. "!", seconds = 5 })
end)

rosorah.on("chat", function(e)
  if e.text == "!brb" and e.author == "YourName" then
    rosorah.run("switch_scene", { scene = "Be Right Back" })
  end
end)
"#;

fn to_lua(lua: &Lua, value: &Value) -> mlua::Result<LuaValue> {
    Ok(match value {
        Value::Null => LuaValue::Nil,
        Value::Bool(b) => LuaValue::Boolean(*b),
        Value::Number(n) => LuaValue::Number(n.as_f64().unwrap_or(0.)),
        Value::String(s) => LuaValue::String(lua.create_string(s)?),
        Value::Array(items) => {
            let t = lua.create_table()?;
            for (i, item) in items.iter().enumerate() {
                t.set(i + 1, to_lua(lua, item)?)?;
            }
            LuaValue::Table(t)
        }
        Value::Object(map) => {
            let t = lua.create_table()?;
            for (k, v) in map {
                t.set(k.as_str(), to_lua(lua, v)?)?;
            }
            LuaValue::Table(t)
        }
    })
}
fn from_lua(value: &LuaValue, depth: u32) -> Value {
    if depth > 8 {
        return Value::Null;
    }
    match value {
        LuaValue::Boolean(b) => Value::Bool(*b),
        LuaValue::Integer(i) => (*i).into(),
        LuaValue::Number(n) => serde_json::Number::from_f64(*n).map_or(Value::Null, Value::Number),
        LuaValue::String(s) => Value::String(s.to_string_lossy()),
        LuaValue::Table(t) => {
            // A table with 1..n keys is a list; otherwise an object.
            let len = t.raw_len();
            if len > 0 {
                Value::Array((1..=len).map(|i| from_lua(&t.raw_get(i).unwrap_or(LuaValue::Nil), depth + 1)).collect())
            } else {
                let mut map = serde_json::Map::new();
                for (k, v) in t.clone().pairs::<LuaValue, LuaValue>().flatten() {
                    if let LuaValue::String(k) = k {
                        map.insert(k.to_string_lossy(), from_lua(&v, depth + 1));
                    }
                }
                Value::Object(map)
            }
        }
        _ => Value::Null,
    }
}
impl Script {
    fn load(path: &Path, name: &str) -> mlua::Result<Self> {
        // Only text, tables, maths, UTF-8 and coroutines: no files, OS or loading other code.
        let lua = Lua::new_with(StdLib::STRING | StdLib::TABLE | StdLib::MATH | StdLib::UTF8 | StdLib::COROUTINE, LuaOptions::default())?;
        lua.set_memory_limit(64 * 1024 * 1024)?;
        let budget = Rc::new(Cell::new(0u32));
        let b = budget.clone();
        // About 2 million instructions per call; a stuck loop stops instead of freezing Rosorah.
        lua.set_hook(HookTriggers::new().every_nth_instruction(10_000), move |_, _| {
            b.set(b.get() + 1);
            if b.get() > 200 {
                Err(mlua::Error::runtime("took too long and was stopped"))
            } else {
                Ok(VmState::Continue)
            }
        });
        let handlers: Rc<RefCell<Vec<(String, RegistryKey)>>> = Rc::default();
        let timers: Rc<RefCell<Vec<Timer>>> = Rc::default();
        let requests: Rc<RefCell<Vec<(String, Value)>>> = Rc::default();
        let log: Rc<RefCell<VecDeque<String>>> = Rc::default();
        let api = lua.create_table()?;
        let h = handlers.clone();
        api.set("on", lua.create_function(move |lua, (event, f): (String, Function)| {
            h.borrow_mut().push((event, lua.create_registry_value(f)?));
            Ok(())
        })?)?;
        let t = timers.clone();
        api.set("every", lua.create_function(move |lua, (seconds, f): (f64, Function)| {
            let every = Duration::from_secs_f64(seconds.clamp(0.1, 86_400.));
            t.borrow_mut().push(Timer { every, next: Instant::now() + every, function: lua.create_registry_value(f)? });
            Ok(())
        })?)?;
        let r = requests.clone();
        api.set("run", lua.create_function(move |_, (command, args): (String, Option<Table>)| {
            let data = args.map_or(Value::Object(Default::default()), |t| from_lua(&LuaValue::Table(t), 0));
            let mut queue = r.borrow_mut();
            if queue.len() < 100 {
                queue.push((command, data));
            }
            Ok(())
        })?)?;
        let l = log.clone();
        api.set("log", lua.create_function(move |_, text: mlua::Variadic<LuaValue>| {
            let line = text.iter().map(|v| match v {
                LuaValue::String(s) => s.to_string_lossy(),
                other => from_lua(other, 0).to_string(),
            }).collect::<Vec<_>>().join(" ");
            let mut log = l.borrow_mut();
            log.push_back(line);
            while log.len() > 50 {
                log.pop_front();
            }
            Ok(())
        })?)?;
        api.set("status_value", lua.create_table()?)?;
        api.set("status", lua.create_function(|lua, ()| lua.globals().get::<Table>("rosorah")?.get::<LuaValue>("status_value"))?)?;
        lua.globals().set("rosorah", api)?;
        let code = std::fs::read_to_string(path).map_err(mlua::Error::external)?;
        budget.set(0);
        lua.load(code).set_name(name).exec()?;
        Ok(Self { name: name.to_owned(), lua, handlers, timers, requests, budget, log })
    }
    fn call(&self, function: &Function, argument: LuaValue) -> mlua::Result<()> {
        self.budget.set(0);
        function.call::<()>(argument)
    }
}
impl Scripts {
    /// Loads the enabled scripts, replacing whatever ran before.
    pub fn load(&mut self, enabled: &[String]) {
        self.running.clear();
        self.errors.clear();
        for name in enabled {
            let path = folder().join(name);
            match Script::load(&path, name) {
                Ok(script) => self.running.push(script),
                Err(e) => {
                    self.errors.insert(name.clone(), e.to_string());
                }
            }
        }
    }
    pub fn log(&self, name: &str) -> Vec<String> {
        self.running.iter().find(|s| s.name == name).map(|s| s.log.borrow().iter().cloned().collect()).unwrap_or_default()
    }
    pub fn is_empty(&self) -> bool {
        self.running.is_empty()
    }
    /// Gives each script this frame's events and due timers; returns the commands they asked for.
    pub fn run(&mut self, events: &[Value], status: &Value) -> Vec<Request> {
        let mut out = Vec::new();
        let now = Instant::now();
        for script in &self.running {
            let mut failure = None;
            if let Ok(api) = script.lua.globals().get::<Table>("rosorah")
                && let Ok(value) = to_lua(&script.lua, status)
            {
                let _ = api.set("status_value", value);
            }
            for event in events {
                let Some(kind) = event["event"].as_str() else { continue };
                let handlers: Vec<Function> = script.handlers.borrow().iter().filter(|(k, _)| k == kind).filter_map(|(_, key)| script.lua.registry_value(key).ok()).collect();
                for f in handlers {
                    let argument = to_lua(&script.lua, event).unwrap_or(LuaValue::Nil);
                    if let Err(e) = script.call(&f, argument) {
                        failure = Some(e.to_string());
                    }
                }
            }
            let due: Vec<Function> = {
                let mut timers = script.timers.borrow_mut();
                timers.iter_mut().filter(|t| t.next <= now).filter_map(|t| {
                    t.next = now + t.every;
                    script.lua.registry_value(&t.function).ok()
                }).collect()
            };
            for f in due {
                if let Err(e) = script.call(&f, LuaValue::Nil) {
                    failure = Some(e.to_string());
                }
            }
            if let Some(e) = failure {
                self.errors.insert(script.name.clone(), e);
            }
            for (command, data) in script.requests.borrow_mut().drain(..) {
                out.push(Request { script: script.name.clone(), command, data });
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn script(code: &str) -> Script {
        let dir = std::env::temp_dir().join(format!("rosorah-scripts-{}-{}", std::process::id(), code.len()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("test.lua");
        std::fs::write(&path, code).unwrap();
        Script::load(&path, "test.lua").unwrap()
    }
    #[test]
    fn events_run_handlers_that_queue_commands() {
        let s = script(r#"rosorah.on("follow", function(e) rosorah.run("alert", { text = "Hi " .. e.author }) end)"#);
        let mut scripts = Scripts { running: vec![s], errors: Default::default() };
        let out = scripts.run(&[serde_json::json!({"event": "follow", "author": "Rose"})], &serde_json::json!({}));
        assert_eq!(out.len(), 1);
        assert_eq!((out[0].command.as_str(), out[0].data["text"].as_str()), ("alert", Some("Hi Rose")));
    }
    #[test]
    fn sandbox_has_no_files_or_system_and_stuck_loops_stop() {
        let s = script(r#"rosorah.on("x", function() while true do end end)
            rosorah.on("y", function() rosorah.log(tostring(io), tostring(os), tostring(require)) end)"#);
        let mut scripts = Scripts { running: vec![s], errors: Default::default() };
        scripts.run(&[serde_json::json!({"event": "y"})], &serde_json::json!({}));
        assert_eq!(scripts.log("test.lua"), vec!["nil nil nil"]);
        scripts.run(&[serde_json::json!({"event": "x"})], &serde_json::json!({}));
        assert!(scripts.errors["test.lua"].contains("took too long"), "{:?}", scripts.errors);
    }
}
