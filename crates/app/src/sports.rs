//! A single bounded, idle-aware score cache shared by every sports widget.
use auroracast_core::widgets::{League, Sports};
use serde_json::Value;
use std::{
    path::PathBuf,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
#[derive(Clone, Debug)]
pub struct Game {
    pub league: League,
    pub home: String,
    pub away: String,
    pub home_name: String,
    pub away_name: String,
    pub score: String,
    pub status: String,
    pub state: u8,
}
#[derive(Default)]
struct Cache {
    key: String,
    games: Vec<Game>,
    status: String,
    updated: Option<Instant>,
    requested: Option<Instant>,
    busy: bool,
    next: Option<Instant>,
}
fn cache() -> &'static Mutex<Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(|| {
        Mutex::new(Cache {
            key: std::fs::read_to_string(key_path())
                .unwrap_or_default()
                .trim()
                .into(),
            ..Default::default()
        })
    })
}
fn key_path() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
        })
        .join("rosorah/sports-token")
}
pub fn connected() -> bool {
    !cache().lock().unwrap().key.is_empty()
}
pub fn configure(key: &str) -> anyhow::Result<()> {
    let path = key_path();
    std::fs::create_dir_all(path.parent().unwrap())?;
    #[cfg(unix)]
    {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&path)?;
        f.write_all(key.trim().as_bytes())?;
    }
    #[cfg(not(unix))]
    std::fs::write(&path, key.trim())?;
    let mut c = cache().lock().unwrap();
    c.key = key.trim().into();
    c.games.clear();
    c.updated = None;
    c.next = None;
    c.status = String::new();
    Ok(())
}
pub fn status() -> String {
    let c = cache().lock().unwrap();
    if c.key.is_empty() {
        "Connect a free BALLDONTLIE API key to load scores".into()
    } else {
        c.status.clone()
    }
}
pub fn snapshot(settings: &Sports) -> (Vec<Game>, String) {
    let mut c = cache().lock().unwrap();
    let now = Instant::now();
    c.requested = Some(now);
    if !c.key.is_empty() && !c.busy && c.next.is_none_or(|t| now >= t) {
        c.busy = true;
        c.next = Some(now + Duration::from_secs(65));
        let key = c.key.clone();
        std::thread::spawn(move || {
            let result = fetch(&key);
            let mut c = cache().lock().unwrap();
            c.busy = false;
            if c.key != key {
                return;
            }
            match result {
                Ok(games) => {
                    c.games = games;
                    c.updated = Some(Instant::now());
                    c.status = "Scores update about once a minute".into();
                }
                Err(e) => {
                    c.status = e;
                    c.next = Some(Instant::now() + Duration::from_secs(180));
                }
            }
        });
    }
    let filters: Vec<_> = settings
        .teams
        .split(',')
        .map(|s| s.trim().to_lowercase())
        .filter(|s| !s.is_empty())
        .collect();
    let games = c
        .games
        .iter()
        .filter(|g| {
            settings.leagues.contains(&g.league)
                && match g.state {
                    0 => settings.upcoming,
                    1 => settings.live,
                    _ => settings.finals,
                }
        })
        .filter(|g| {
            filters.is_empty()
                || filters.iter().any(|t| {
                    [&g.home, &g.away, &g.home_name, &g.away_name]
                        .iter()
                        .any(|s| s.to_lowercase().contains(t))
                })
        })
        .cloned()
        .collect();
    let status = match c.updated {
        Some(t) if t.elapsed().as_secs() > 150 => {
            format!("Stale · last update {} min ago", t.elapsed().as_secs() / 60)
        }
        Some(t) => format!("Updated {}s ago", t.elapsed().as_secs() / 10 * 10),
        None => {
            if c.key.is_empty() {
                "Connect sports in widget settings".into()
            } else if c.busy {
                "Loading scores…".into()
            } else {
                c.status.clone()
            }
        }
    };
    (games, status)
}
fn fetch(key: &str) -> Result<Vec<Game>, String> {
    let client = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(12))
        .build()
        .map_err(|_| "Could not create score connection")?;
    let now = gst::glib::DateTime::now_local().map_err(|_| "System date unavailable")?;
    let dates = [
        now.add_days(-1).unwrap(),
        now.clone(),
        now.add_days(1).unwrap(),
    ];
    let mut all = Vec::new();
    for (league, path) in [(League::Nba, "v1/games"), (League::Nfl, "nfl/v1/games")] {
        let mut params = vec![("per_page".to_string(), "100".to_string())];
        for date in &dates {
            params.push((
                "dates[]".into(),
                date.format("%Y-%m-%d").unwrap().to_string(),
            ));
        }
        let r = client
            .get(format!("https://api.balldontlie.io/{path}"))
            .header("Authorization", key)
            .query(&params)
            .send()
            .map_err(|_| "Sports service unavailable; retaining last scores")?;
        if !r.status().is_success() {
            return Err(match r.status().as_u16() {
                401 | 403 => "Sports key rejected. Check your BALLDONTLIE key.",
                429 => "Sports request limit reached. Retrying in three minutes.",
                _ => "Sports service unavailable. Retrying in three minutes.",
            }
            .into());
        }
        let data: Value = r.json().map_err(|_| "Invalid sports response")?;
        for g in data["data"]
            .as_array()
            .ok_or("Missing games in sports response")?
            .iter()
            .take(100)
        {
            if let Some(g) = parse(league, g) {
                all.push(g)
            }
        }
    }
    Ok(all)
}
fn parse(league: League, g: &Value) -> Option<Game> {
    let s = |v: &Value, k: &str| v[k].as_str().unwrap_or("").to_string();
    let home = s(&g["home_team"], "abbreviation");
    let away = s(&g["visitor_team"], "abbreviation");
    if home.is_empty() || away.is_empty() {
        return None;
    }
    let status = s(g, "status");
    let lower = status.to_lowercase();
    let state = if lower.contains("final") || s(g, "status_state") == "post" {
        2
    } else if g["period"].as_u64().unwrap_or(0) > 0 || s(g, "status_state") == "in" {
        1
    } else {
        0
    };
    let score = if state == 0 {
        "vs".into()
    } else {
        format!(
            "{} – {}",
            g["visitor_team_score"].as_u64().unwrap_or(0),
            g["home_team_score"].as_u64().unwrap_or(0)
        )
    };
    Some(Game {
        league,
        home,
        away,
        home_name: s(&g["home_team"], "full_name"),
        away_name: s(&g["visitor_team"], "full_name"),
        score,
        status,
        state,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_final_without_live_clock() {
        let g=parse(League::Nba,&serde_json::json!({"home_team":{"abbreviation":"BOS"},"visitor_team":{"abbreviation":"NYK"},"home_team_score":105,"visitor_team_score":98,"status":"Final","period":4})).unwrap();
        assert_eq!(g.state, 2);
        assert_eq!(g.score, "98 – 105");
    }
}
