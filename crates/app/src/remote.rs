//! Remote control over WebSocket. One server speaks two languages, chosen by what the controller
//! asks for when it connects: Rosorah's own protocol (full control and events) or OBS's
//! published obs-websocket 5 protocol, so Stream Deck, Touch Portal and phone apps made for OBS
//! work unchanged. Written from the protocol description; requests are carried out by the
//! studio on its own update loop (see `Call`).
use anyhow::{Context, Result};
use base64::Engine as _;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    io::ErrorKind,
    net::{TcpListener, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
        mpsc,
    },
    time::Duration,
};
use tungstenite::{Message, WebSocket, handshake::server::{Request, Response}};

/// A command for the studio to carry out, with where to send the answer.
pub struct Call {
    pub command: String,
    pub data: Value,
    pub reply: mpsc::Sender<Result<Value, String>>,
}
/// The server, while it runs. Dropping it stops it.
pub struct Server {
    pub calls: mpsc::Receiver<Call>,
    clients: Arc<Mutex<Vec<mpsc::Sender<Value>>>>,
    stop: Arc<AtomicBool>,
    pub connected: Arc<AtomicUsize>,
    pub port: u16,
}
impl Drop for Server {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}
impl Server {
    /// Listens on `port`, on this computer only unless `network` is set.
    pub fn start(port: u16, network: bool, password: String) -> Result<Self> {
        let address = if network { "0.0.0.0" } else { "127.0.0.1" };
        let listener = TcpListener::bind((address, port)).with_context(|| format!("Port {port} is in use (is OBS's WebSocket server running?)"))?;
        listener.set_nonblocking(true)?;
        let (tx, calls) = mpsc::channel();
        let clients: Arc<Mutex<Vec<mpsc::Sender<Value>>>> = Arc::default();
        let stop = Arc::new(AtomicBool::new(false));
        let connected = Arc::new(AtomicUsize::new(0));
        let (c, s, n) = (clients.clone(), stop.clone(), connected.clone());
        std::thread::spawn(move || {
            while !s.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((stream, _)) => {
                        let (events_tx, events) = mpsc::channel();
                        c.lock().unwrap().push(events_tx);
                        let (tx, s, n, password) = (tx.clone(), s.clone(), n.clone(), password.clone());
                        std::thread::spawn(move || {
                            n.fetch_add(1, Ordering::Relaxed);
                            let _ = serve(stream, &password, &tx, &events, &s);
                            n.fetch_sub(1, Ordering::Relaxed);
                        });
                    }
                    Err(e) if e.kind() == ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(100)),
                    Err(_) => std::thread::sleep(Duration::from_millis(500)),
                }
            }
        });
        Ok(Self { calls, clients, stop, connected, port })
    }
    /// Sends a Rosorah event to every connected controller (each translates it for its protocol).
    pub fn broadcast(&self, event: Value) {
        self.clients.lock().unwrap().retain(|c| c.send(event.clone()).is_ok());
    }
}

/// OBS's handshake hashes: base64(sha256(password + salt)), then base64(sha256(secret + challenge)).
fn obs_auth(password: &str, salt: &str, challenge: &str) -> String {
    let b64 = base64::engine::general_purpose::STANDARD;
    let secret = b64.encode(Sha256::digest(format!("{password}{salt}").as_bytes()));
    b64.encode(Sha256::digest(format!("{secret}{challenge}").as_bytes()))
}
fn random_text() -> String {
    let mut bytes = [0u8; 24];
    let seed = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos() ^ (std::process::id() as u128) << 64;
    let mut x = seed as u64 | 1;
    for b in &mut bytes {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        *b = x as u8;
    }
    base64::engine::general_purpose::STANDARD.encode(bytes)
}
/// A new random password for the remote-control server.
pub fn new_password() -> String {
    random_text().chars().filter(|c| c.is_ascii_alphanumeric()).take(16).collect()
}

fn serve(stream: TcpStream, password: &str, calls: &mpsc::Sender<Call>, events: &mpsc::Receiver<Value>, stop: &AtomicBool) -> Result<()> {
    stream.set_nonblocking(false)?;
    let mut obs = false;
    let mut socket: WebSocket<TcpStream> = tungstenite::accept_hdr(stream, |request: &Request, mut response: Response| {
        // OBS tools ask for "obswebsocket.json"; anyone else gets Rosorah's protocol.
        let wanted = request.headers().get("Sec-WebSocket-Protocol").and_then(|v| v.to_str().ok()).unwrap_or("");
        if wanted.split(',').any(|p| p.trim() == "obswebsocket.json") {
            obs = true;
            response.headers_mut().insert("Sec-WebSocket-Protocol", "obswebsocket.json".parse().unwrap());
        } else if wanted.split(',').any(|p| p.trim() == "rosorah.v1") {
            response.headers_mut().insert("Sec-WebSocket-Protocol", "rosorah.v1".parse().unwrap());
        }
        Ok(response)
    })
    .map_err(|e| anyhow::anyhow!("{e}"))?;
    socket.get_ref().set_read_timeout(Some(Duration::from_millis(50)))?;
    let (salt, challenge) = (random_text(), random_text());
    let mut identified = false;
    if obs {
        let mut hello = json!({"op": 0, "d": {"obsWebSocketVersion": "5.5.0", "rpcVersion": 1}});
        if !password.is_empty() {
            hello["d"]["authentication"] = json!({"challenge": challenge, "salt": salt});
        }
        socket.send(Message::text(hello.to_string()))?;
    } else {
        socket.send(Message::text(json!({"hello": "Rosorah Studio", "version": env!("CARGO_PKG_VERSION"), "password": !password.is_empty()}).to_string()))?;
        identified = password.is_empty();
    }
    let ask = |command: &str, data: Value| -> Result<Value, String> {
        let (reply, answer) = mpsc::channel();
        calls.send(Call { command: command.into(), data, reply }).map_err(|_| "Rosorah is closing".to_string())?;
        answer.recv_timeout(Duration::from_secs(10)).map_err(|_| "Rosorah didn't answer in time".to_string())?
    };
    while !stop.load(Ordering::Relaxed) {
        // Events go out between messages.
        while let Ok(event) = events.try_recv() {
            if !identified {
                continue;
            }
            let out = if obs { obs_event(&event) } else { Some(event) };
            if let Some(out) = out {
                socket.send(Message::text(out.to_string()))?;
            }
        }
        let message = match socket.read() {
            Ok(m) => m,
            Err(tungstenite::Error::Io(e)) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => continue,
            Err(_) => break,
        };
        let Message::Text(text) = message else {
            if matches!(message, Message::Close(_)) {
                break;
            }
            continue;
        };
        let Ok(input) = serde_json::from_str::<Value>(&text) else { continue };
        if obs {
            match input["op"].as_u64() {
                Some(1) => {
                    if !password.is_empty() && input["d"]["authentication"].as_str() != Some(obs_auth(password, &salt, &challenge).as_str()) {
                        let _ = socket.close(Some(tungstenite::protocol::CloseFrame { code: 4009.into(), reason: "Authentication failed".into() }));
                        break;
                    }
                    identified = true;
                    socket.send(Message::text(json!({"op": 2, "d": {"negotiatedRpcVersion": 1}}).to_string()))?;
                }
                Some(6) if identified => {
                    let d = &input["d"];
                    let out = obs_request(d["requestType"].as_str().unwrap_or(""), &d["requestData"], &ask);
                    socket.send(Message::text(json!({"op": 7, "d": {"requestType": d["requestType"], "requestId": d["requestId"], "requestStatus": out.0, "responseData": out.1}}).to_string()))?;
                }
                Some(8) if identified => {
                    let d = &input["d"];
                    let results: Vec<Value> = d["requests"].as_array().into_iter().flatten().map(|r| {
                        let out = obs_request(r["requestType"].as_str().unwrap_or(""), &r["requestData"], &ask);
                        json!({"requestType": r["requestType"], "requestId": r["requestId"], "requestStatus": out.0, "responseData": out.1})
                    }).collect();
                    socket.send(Message::text(json!({"op": 9, "d": {"requestId": d["requestId"], "results": results}}).to_string()))?;
                }
                _ => {}
            }
            continue;
        }
        // Rosorah protocol: {"id": 1, "do": "switch_scene", "scene": "Gaming"} → {"id": 1, "ok": true, ...}
        let id = input["id"].clone();
        let command = input["do"].as_str().unwrap_or("").to_owned();
        let answer = if command == "hello" {
            identified = password.is_empty() || input["password"].as_str() == Some(password);
            if identified { Ok(json!({"welcome": true})) } else { Err("Wrong password".into()) }
        } else if !identified {
            Err("Send {\"do\": \"hello\", \"password\": \"…\"} first".into())
        } else {
            ask(&command, input.clone())
        };
        let reply = match answer {
            Ok(mut data) => {
                if let Value::Object(map) = &mut data {
                    map.insert("id".into(), id);
                    map.insert("ok".into(), true.into());
                    data
                } else {
                    json!({"id": id, "ok": true, "result": data})
                }
            }
            Err(e) => json!({"id": id, "ok": false, "error": e}),
        };
        socket.send(Message::text(reply.to_string()))?;
    }
    Ok(())
}

/// An OBS request answered through Rosorah's commands: (requestStatus, responseData).
fn obs_request(kind: &str, data: &Value, ask: &dyn Fn(&str, Value) -> Result<Value, String>) -> (Value, Value) {
    let ok = |d: Value| (json!({"result": true, "code": 100}), d);
    let fail = |code: u32, comment: String| (json!({"result": false, "code": code, "comment": comment}), Value::Null);
    let call = |command: &str, args: Value| ask(command, args);
    let scene_arg = || data.get("sceneName").or(data.get("sceneUuid")).cloned().unwrap_or(Value::Null);
    let sound_arg = || data.get("inputName").or(data.get("inputUuid")).cloned().unwrap_or(Value::Null);
    let result = (|| -> Result<Value, String> {
        Ok(match kind {
            "GetVersion" => json!({"obsVersion": "30.0.0", "obsWebSocketVersion": "5.5.0", "rpcVersion": 1, "platform": std::env::consts::OS, "platformDescription": "Rosorah Studio", "supportedImageFormats": [], "availableRequests": OBS_REQUESTS}),
            "GetStats" => json!({"cpuUsage": 0, "memoryUsage": 0, "activeFps": 60, "renderSkippedFrames": 0, "renderTotalFrames": 0, "outputSkippedFrames": 0, "outputTotalFrames": 0, "webSocketSessionIncomingMessages": 0, "webSocketSessionOutgoingMessages": 0}),
            "GetSceneList" => {
                let s = call("scenes", json!({}))?;
                let list = s["scenes"].as_array().cloned().unwrap_or_default();
                let n = list.len();
                // OBS numbers scenes from the bottom of the list.
                let scenes: Vec<Value> = list.iter().enumerate().map(|(i, sc)| json!({"sceneName": sc["name"], "sceneUuid": sc["id"].to_string(), "sceneIndex": n - 1 - i})).collect();
                json!({"currentProgramSceneName": s["live"]["name"], "currentProgramSceneUuid": s["live"]["id"].to_string(), "currentPreviewSceneName": s["preview"]["name"], "currentPreviewSceneUuid": s["preview"]["id"].to_string(), "scenes": scenes})
            }
            "GetCurrentProgramScene" => {
                let s = call("scenes", json!({}))?;
                json!({"currentProgramSceneName": s["live"]["name"], "sceneName": s["live"]["name"], "sceneUuid": s["live"]["id"].to_string(), "currentProgramSceneUuid": s["live"]["id"].to_string()})
            }
            "SetCurrentProgramScene" => call("switch_scene", json!({"scene": scene_arg(), "live": true}))?.pipe(|_| json!({})),
            "GetCurrentPreviewScene" => {
                let s = call("scenes", json!({}))?;
                if s["preview"].is_null() {
                    return Err("Studio Mode isn't on".into());
                }
                json!({"currentPreviewSceneName": s["preview"]["name"], "sceneName": s["preview"]["name"], "sceneUuid": s["preview"]["id"].to_string()})
            }
            "SetCurrentPreviewScene" => call("preview_scene", json!({"scene": scene_arg()}))?.pipe(|_| json!({})),
            "GetStudioModeEnabled" => json!({"studioModeEnabled": call("status", json!({}))?["studio_mode"]}),
            "SetStudioModeEnabled" => call("studio_mode", json!({"on": data["studioModeEnabled"]}))?.pipe(|_| json!({})),
            "TriggerStudioModeTransition" => call("transition", json!({}))?.pipe(|_| json!({})),
            "GetStreamStatus" => {
                let s = call("status", json!({}))?;
                json!({"outputActive": s["live"], "outputReconnecting": false, "outputTimecode": "00:00:00.000", "outputDuration": 0, "outputCongestion": 0, "outputBytes": 0, "outputSkippedFrames": 0, "outputTotalFrames": 0})
            }
            "StartStream" => call("go_live", json!({}))?.pipe(|_| json!({})),
            "StopStream" => call("stop_live", json!({}))?.pipe(|_| json!({})),
            "ToggleStream" => {
                let live = call("status", json!({}))?["live"].as_bool().unwrap_or(false);
                call(if live { "stop_live" } else { "go_live" }, json!({}))?;
                json!({"outputActive": !live})
            }
            "GetRecordStatus" => {
                let s = call("status", json!({}))?;
                json!({"outputActive": s["recording"], "outputPaused": false, "outputTimecode": "00:00:00.000", "outputDuration": 0, "outputBytes": 0})
            }
            "StartRecord" => call("record", json!({"on": true}))?.pipe(|_| json!({})),
            "StopRecord" => call("record", json!({"on": false}))?.pipe(|_| json!({"outputPath": ""})),
            "ToggleRecord" => json!({"outputActive": call("record", json!({}))?["recording"]}),
            "GetReplayBufferStatus" => json!({"outputActive": call("status", json!({}))?["replay"]}),
            "StartReplayBuffer" => call("replay", json!({"on": true}))?.pipe(|_| json!({})),
            "StopReplayBuffer" => call("replay", json!({"on": false}))?.pipe(|_| json!({})),
            "ToggleReplayBuffer" => json!({"outputActive": call("replay", json!({}))?["replay"]}),
            "SaveReplayBuffer" => call("save_replay", json!({}))?.pipe(|_| json!({})),
            "GetVirtualCamStatus" => json!({"outputActive": call("status", json!({}))?["virtual_camera"]}),
            "StartVirtualCam" => call("virtual_camera", json!({"on": true}))?.pipe(|_| json!({})),
            "StopVirtualCam" => call("virtual_camera", json!({"on": false}))?.pipe(|_| json!({})),
            "ToggleVirtualCam" => json!({"outputActive": call("virtual_camera", json!({}))?["virtual_camera"]}),
            "GetInputList" => {
                let s = call("sounds", json!({}))?;
                let inputs: Vec<Value> = s["sounds"].as_array().into_iter().flatten().map(|x| json!({"inputName": x["name"], "inputUuid": x["id"].to_string(), "inputKind": "rosorah_sound", "unversionedInputKind": "rosorah_sound"})).collect();
                json!({"inputs": inputs})
            }
            "GetInputMute" => json!({"inputMuted": call("sound", json!({"sound": sound_arg()}))?["muted"]}),
            "SetInputMute" => call("mute", json!({"sound": sound_arg(), "on": data["inputMuted"]}))?.pipe(|_| json!({})),
            "ToggleInputMute" => json!({"inputMuted": call("mute", json!({"sound": sound_arg()}))?["muted"]}),
            "GetInputVolume" => {
                let s = call("sound", json!({"sound": sound_arg()}))?;
                let db = s["volume_db"].as_f64().unwrap_or(0.);
                json!({"inputVolumeDb": db, "inputVolumeMul": 10f64.powf(db / 20.)})
            }
            "SetInputVolume" => {
                let db = data["inputVolumeDb"].as_f64().or_else(|| data["inputVolumeMul"].as_f64().map(|m| 20. * m.max(1e-6).log10())).ok_or("Give inputVolumeDb or inputVolumeMul")?;
                call("volume", json!({"sound": sound_arg(), "db": db}))?.pipe(|_| json!({}))
            }
            "GetSceneItemList" => {
                let s = call("layers", json!({"scene": scene_arg()}))?;
                let items: Vec<Value> = s["layers"].as_array().into_iter().flatten().map(|l| json!({"sceneItemId": l["index"].as_u64().unwrap_or(0) + 1, "sceneItemIndex": l["index"], "sourceName": l["name"], "sceneItemEnabled": l["visible"], "sourceUuid": l["source"].to_string()})).collect();
                json!({"sceneItems": items})
            }
            "GetSceneItemId" => {
                let s = call("layers", json!({"scene": scene_arg()}))?;
                let name = data["sourceName"].as_str().unwrap_or("");
                let found = s["layers"].as_array().into_iter().flatten().find(|l| l["name"].as_str() == Some(name)).ok_or("No layer with that name")?;
                json!({"sceneItemId": found["index"].as_u64().unwrap_or(0) + 1})
            }
            "GetSceneItemEnabled" => {
                let s = call("layers", json!({"scene": scene_arg()}))?;
                let index = data["sceneItemId"].as_u64().unwrap_or(0).saturating_sub(1);
                let found = s["layers"].as_array().into_iter().flatten().find(|l| l["index"].as_u64() == Some(index)).ok_or("No such scene item")?;
                json!({"sceneItemEnabled": found["visible"]})
            }
            "SetSceneItemEnabled" => {
                let index = data["sceneItemId"].as_u64().unwrap_or(0).saturating_sub(1);
                call("layer", json!({"scene": scene_arg(), "layer": index, "visible": data["sceneItemEnabled"]}))?.pipe(|_| json!({}))
            }
            _ => return Err(format!("unknown:{kind}")),
        })
    })();
    match result {
        Ok(d) => ok(d),
        Err(e) if e.starts_with("unknown:") => fail(204, format!("Rosorah Studio doesn't support {kind}")),
        Err(e) => fail(600, e),
    }
}
/// Requests answered in OBS mode.
const OBS_REQUESTS: &[&str] = &[
    "GetVersion", "GetStats", "GetSceneList", "GetCurrentProgramScene", "SetCurrentProgramScene", "GetCurrentPreviewScene", "SetCurrentPreviewScene",
    "GetStudioModeEnabled", "SetStudioModeEnabled", "TriggerStudioModeTransition", "GetStreamStatus", "StartStream", "StopStream", "ToggleStream",
    "GetRecordStatus", "StartRecord", "StopRecord", "ToggleRecord", "GetReplayBufferStatus", "StartReplayBuffer", "StopReplayBuffer",
    "ToggleReplayBuffer", "SaveReplayBuffer", "GetVirtualCamStatus", "StartVirtualCam", "StopVirtualCam", "ToggleVirtualCam", "GetInputList",
    "GetInputMute", "SetInputMute", "ToggleInputMute", "GetInputVolume", "SetInputVolume", "GetSceneItemList", "GetSceneItemId",
    "GetSceneItemEnabled", "SetSceneItemEnabled",
];
/// A Rosorah event in OBS's form, when OBS has one.
fn obs_event(event: &Value) -> Option<Value> {
    let (kind, data) = match event["event"].as_str()? {
        "scene" => ("CurrentProgramSceneChanged", json!({"sceneName": event["name"], "sceneUuid": event["id"].to_string()})),
        "preview" => ("CurrentPreviewSceneChanged", json!({"sceneName": event["name"], "sceneUuid": event["id"].to_string()})),
        "studio_mode" => ("StudioModeStateChanged", json!({"studioModeEnabled": event["on"]})),
        "live" => ("StreamStateChanged", json!({"outputActive": event["on"], "outputState": if event["on"] == true { "OBS_WEBSOCKET_OUTPUT_STARTED" } else { "OBS_WEBSOCKET_OUTPUT_STOPPED" }})),
        "recording" => ("RecordStateChanged", json!({"outputActive": event["on"], "outputState": if event["on"] == true { "OBS_WEBSOCKET_OUTPUT_STARTED" } else { "OBS_WEBSOCKET_OUTPUT_STOPPED" }})),
        "replay" => ("ReplayBufferStateChanged", json!({"outputActive": event["on"], "outputState": if event["on"] == true { "OBS_WEBSOCKET_OUTPUT_STARTED" } else { "OBS_WEBSOCKET_OUTPUT_STOPPED" }})),
        "virtual_camera" => ("VirtualcamStateChanged", json!({"outputActive": event["on"], "outputState": if event["on"] == true { "OBS_WEBSOCKET_OUTPUT_STARTED" } else { "OBS_WEBSOCKET_OUTPUT_STOPPED" }})),
        "mute" => ("InputMuteStateChanged", json!({"inputName": event["name"], "inputUuid": event["id"].to_string(), "inputMuted": event["muted"]})),
        _ => return None,
    };
    Some(json!({"op": 5, "d": {"eventType": kind, "eventIntent": 0, "eventData": data}}))
}
/// Lets a result be turned into another value in a chain.
trait Pipe: Sized {
    fn pipe<T>(self, f: impl FnOnce(Self) -> T) -> T {
        f(self)
    }
}
impl Pipe for Value {}

#[cfg(test)]
mod tests {
    use super::*;
    use tungstenite::client::IntoClientRequest;
    /// A studio stand-in: answers "scenes" with two scenes, anything else with {}.
    fn answer_calls(server: &Server) {
        while let Ok(call) = server.calls.recv_timeout(Duration::from_millis(500)) {
            let reply = match call.command.as_str() {
                "scenes" => json!({"scenes": [{"id": 1, "name": "Gaming"}, {"id": 2, "name": "BRB"}], "live": {"id": 1, "name": "Gaming"}, "preview": null}),
                _ => json!({}),
            };
            let _ = call.reply.send(Ok(reply));
        }
    }
    fn free_port() -> u16 {
        TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port()
    }
    fn read_json(socket: &mut WebSocket<tungstenite::stream::MaybeTlsStream<TcpStream>>) -> Value {
        loop {
            if let Message::Text(t) = socket.read().unwrap() {
                return serde_json::from_str(&t).unwrap();
            }
        }
    }
    #[test]
    fn obs_tools_log_in_and_list_scenes() {
        let port = free_port();
        let server = Server::start(port, false, "letmein".into()).unwrap();
        let client = std::thread::spawn(move || {
            let mut request = format!("ws://127.0.0.1:{port}").into_client_request().unwrap();
            request.headers_mut().insert("Sec-WebSocket-Protocol", "obswebsocket.json".parse().unwrap());
            let (mut socket, response) = tungstenite::connect(request).unwrap();
            assert_eq!(response.headers().get("Sec-WebSocket-Protocol").unwrap(), "obswebsocket.json");
            let hello = read_json(&mut socket);
            assert_eq!(hello["op"], 0);
            let auth = &hello["d"]["authentication"];
            let answer = obs_auth("letmein", auth["salt"].as_str().unwrap(), auth["challenge"].as_str().unwrap());
            socket.send(Message::text(json!({"op": 1, "d": {"rpcVersion": 1, "authentication": answer}}).to_string())).unwrap();
            assert_eq!(read_json(&mut socket)["op"], 2, "identified");
            socket.send(Message::text(json!({"op": 6, "d": {"requestType": "GetSceneList", "requestId": "a"}}).to_string())).unwrap();
            let reply = read_json(&mut socket);
            assert_eq!(reply["op"], 7);
            assert_eq!(reply["d"]["requestStatus"]["result"], true);
            assert_eq!(reply["d"]["responseData"]["currentProgramSceneName"], "Gaming");
            assert_eq!(reply["d"]["responseData"]["scenes"].as_array().unwrap().len(), 2);
            socket.send(Message::text(json!({"op": 6, "d": {"requestType": "MadeUpRequest", "requestId": "b"}}).to_string())).unwrap();
            assert_eq!(read_json(&mut socket)["d"]["requestStatus"]["code"], 204, "unknown requests say so");
        });
        answer_calls(&server);
        client.join().unwrap();
    }
    #[test]
    fn rosorah_protocol_needs_the_password_and_gets_events() {
        let port = free_port();
        let server = Server::start(port, false, "letmein".into()).unwrap();
        let client = std::thread::spawn(move || {
            let (mut socket, _) = tungstenite::connect(format!("ws://127.0.0.1:{port}")).unwrap();
            assert_eq!(read_json(&mut socket)["password"], true);
            socket.send(Message::text(json!({"id": 1, "do": "scenes"}).to_string())).unwrap();
            assert_eq!(read_json(&mut socket)["ok"], false, "refused before hello");
            socket.send(Message::text(json!({"id": 2, "do": "hello", "password": "letmein"}).to_string())).unwrap();
            assert_eq!(read_json(&mut socket)["ok"], true);
            socket.send(Message::text(json!({"id": 3, "do": "scenes"}).to_string())).unwrap();
            let reply = read_json(&mut socket);
            assert_eq!((reply["id"].as_u64(), reply["ok"].as_bool()), (Some(3), Some(true)));
            // The test below pushes a scene event once we're in.
            let event = read_json(&mut socket);
            assert_eq!(event["event"], "scene");
        });
        answer_calls(&server);
        std::thread::sleep(Duration::from_millis(200));
        server.broadcast(json!({"event": "scene", "id": 2, "name": "BRB"}));
        client.join().unwrap();
    }
    #[test]
    fn obs_authentication_matches_the_published_example() {
        // Same password, salt and challenge give the same answer both ways, and differ otherwise.
        let a = obs_auth("supersecretpassword", "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=", "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY=");
        assert_eq!(a, obs_auth("supersecretpassword", "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=", "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY="));
        assert_eq!(a, "1Ct943GAT+6YQUUX47Ia/ncufilbe6+oD6lY+5kaCu4=");
        assert_ne!(a, obs_auth("other", "lM1GncleQOaCu9lT1yeUZhFYnqhsLLP1G5lAGo3ixaI=", "+IxH4CnCiqpX1rM9scsNynZzbOe4KhDeYcTNS3PDaeY="));
    }
}
