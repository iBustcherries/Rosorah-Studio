//! Match audio to the renderer's host PID and its descendants. Never use a
//! title/name match or Steam's common launcher as an audio source selector.
use rosorah_capture::GameProcess;
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    os::unix::fs::MetadataExt,
};

pub const CHANNEL: u64 = 10;

#[derive(Clone, Copy)]
struct Process {
    parent: u32,
    start_time: u64,
}

fn parse_stat(stat: &str) -> Option<Process> {
    // comm can contain spaces and parentheses; fields after its final ')' are
    // stable. Field 22 is the kernel start time, in clock ticks since boot.
    let fields: Vec<_> = stat.rsplit_once(')')?.1.split_whitespace().collect();
    Some(Process {
        parent: fields.get(1)?.parse().ok()?,
        start_time: fields.get(19)?.parse().ok()?,
    })
}

fn process(pid: u32) -> Option<Process> {
    let path = format!("/proc/{pid}");
    if fs::metadata(&path).ok()?.uid() != unsafe { libc::geteuid() } {
        return None;
    }
    parse_stat(&fs::read_to_string(format!("{path}/stat")).ok()?)
}

pub fn identity(pid: u32, name: String) -> Option<GameProcess> {
    Some(GameProcess {
        pid,
        start_time: process(pid)?.start_time,
        name,
    })
}

pub fn alive(game: &GameProcess) -> bool {
    process(game.pid).is_some_and(|p| p.start_time == game.start_time)
}

fn related(game: &GameProcess, rows: &BTreeMap<u32, Process>) -> BTreeSet<u32> {
    if rows
        .get(&game.pid)
        .is_none_or(|p| p.start_time != game.start_time)
    {
        return BTreeSet::new();
    }
    rows.keys()
        .copied()
        .filter(|candidate| {
            let mut pid = *candidate;
            for _ in 0..64 {
                if pid == game.pid {
                    return true;
                }
                let Some(p) = rows.get(&pid) else {
                    return false;
                };
                if p.start_time < game.start_time || p.parent == pid {
                    return false;
                }
                pid = p.parent;
            }
            false
        })
        .collect()
}

pub fn family(game: &GameProcess) -> BTreeSet<u32> {
    let Ok(entries) = fs::read_dir("/proc") else {
        return BTreeSet::new();
    };
    let rows = entries
        .flatten()
        .filter_map(|p| {
            let pid = p.file_name().to_str()?.parse::<u32>().ok()?;
            Some((pid, process(pid)?))
        })
        .collect();
    related(game, &rows)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn game_and_children_only_not_steam_or_sibling_music() {
        let mut rows = BTreeMap::new();
        for (pid, parent, start_time) in [
            (1, 0, 1),
            (100, 1, 2),
            (200, 100, 10),
            (201, 200, 11),
            (202, 201, 12),
            (300, 100, 13),
        ] {
            rows.insert(pid, Process { parent, start_time });
        }
        let game = GameProcess {
            pid: 200,
            start_time: 10,
            name: "game".into(),
        };
        assert_eq!(related(&game, &rows), BTreeSet::from([200, 201, 202]));
        rows.get_mut(&200).unwrap().start_time = 20;
        assert!(
            related(&game, &rows).is_empty(),
            "PID reuse must not capture a replacement app"
        );
    }
    #[test]
    fn proc_stat_handles_parentheses_in_process_names() {
        let mut fields = vec!["0"; 20];
        fields[0] = "S";
        fields[1] = "42";
        fields[19] = "123456";
        let p = parse_stat(&format!("77 (game ) with spaces) {}", fields.join(" "))).unwrap();
        assert_eq!((p.parent, p.start_time), (42, 123456));
        assert!(parse_stat("77 (truncated) S 42").is_none());
    }
}
