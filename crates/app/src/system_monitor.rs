//! One background sampler, one replaceable snapshot; no telemetry I/O on the UI thread.
use std::{
    sync::{Arc, Mutex, mpsc},
    time::Duration,
};
#[derive(Clone, Default)]
struct Sample {
    cpu: Option<f32>,
    ram: Option<f32>,
    gpu: Vec<(String, u32)>,
}
pub struct Resources {
    pub cpu: Option<f32>,
    pub ram: Option<f32>,
    pub gpu: Vec<(String, u32)>,
    latest: Arc<Mutex<Sample>>,
    stop: mpsc::Sender<()>,
}
impl Default for Resources {
    fn default() -> Self {
        let latest = Arc::new(Mutex::new(Sample::default()));
        let shared = latest.clone();
        let (stop, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let mut previous = None;
            #[allow(clippy::default_constructed_unit_structs)] // Empty on Linux, not on Windows.
            let mut platform = Platform::default();
            loop {
                let (ticks, ram, gpu) = platform.read();
                let cpu = ticks.and_then(|now| {
                    let value = previous.and_then(|old| cpu_delta(old, now));
                    previous = Some(now);
                    value
                });
                *shared.lock().unwrap() = Sample { cpu, ram, gpu };
                if rx.recv_timeout(Duration::from_secs(1)) != Err(mpsc::RecvTimeoutError::Timeout) {
                    break;
                }
            }
        });
        Self {
            cpu: None,
            ram: None,
            gpu: Vec::new(),
            latest,
            stop,
        }
    }
}
impl Resources {
    pub fn sample(&mut self) {
        let s = self.latest.lock().unwrap().clone();
        self.cpu = s.cpu;
        self.ram = s.ram;
        self.gpu = s.gpu;
    }
}
impl Drop for Resources {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}
fn cpu_delta(old: (u64, u64), new: (u64, u64)) -> Option<f32> {
    let total = new.0.checked_sub(old.0)?;
    let idle = new.1.checked_sub(old.1)?;
    (total > 0).then(|| (100. * total.saturating_sub(idle) as f32 / total as f32).clamp(0., 100.))
}
#[cfg(not(windows))]
#[derive(Default)]
struct Platform;
#[cfg(not(windows))]
impl Platform {
    fn read(&mut self) -> (Option<(u64, u64)>, Option<f32>, Vec<(String, u32)>) {
        let ticks = std::fs::read_to_string("/proc/stat")
            .ok()
            .and_then(|s| parse_cpu(&s));
        let ram = std::fs::read_to_string("/proc/meminfo")
            .ok()
            .and_then(|s| parse_memory(&s));
        let gpu = std::fs::read_dir("/sys/class/drm")
            .into_iter()
            .flatten()
            .filter_map(Result::ok)
            .filter_map(|entry| {
                let name = entry.file_name().to_string_lossy().into_owned();
                if !name.starts_with("card") || !name[4..].chars().all(|c| c.is_ascii_digit()) {
                    return None;
                }
                let v = std::fs::read_to_string(entry.path().join("device/gpu_busy_percent"))
                    .ok()?
                    .trim()
                    .parse::<u32>()
                    .ok()?;
                Some((name, v.min(100)))
            })
            .collect();
        (ticks, ram, gpu)
    }
}
#[cfg(any(not(windows), test))]
fn parse_cpu(s: &str) -> Option<(u64, u64)> {
    let mut words = s.lines().next()?.split_whitespace();
    if words.next()? != "cpu" {
        return None;
    }
    let v: Vec<u64> = words
        .take(8)
        .map(str::parse)
        .collect::<Result<_, _>>()
        .ok()?;
    if v.len() < 4 {
        return None;
    }
    Some((v.iter().sum(), v[3] + v.get(4).copied().unwrap_or(0)))
}
#[cfg(any(not(windows), test))]
fn parse_memory(s: &str) -> Option<f32> {
    let get = |key: &str| {
        s.lines()
            .find(|l| l.starts_with(key))
            .and_then(|l| l.split_whitespace().nth(1))?
            .parse::<u64>()
            .ok()
    };
    let total = get("MemTotal:")?;
    let available = get("MemAvailable:")?;
    (total > 0).then(|| 100. * total.saturating_sub(available) as f32 / total as f32)
}
#[cfg(windows)]
use win::Platform;
#[cfg(windows)]
mod win {
    use std::{collections::BTreeMap, ffi::c_void, ptr};
    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }
    impl FileTime {
        fn ticks(&self) -> u64 {
            (self.high as u64) << 32 | self.low as u64
        }
    }
    #[repr(C)]
    #[derive(Default)]
    struct Memory {
        length: u32,
        load: u32,
        total: u64,
        available: u64,
        page: u64,
        available_page: u64,
        virtual_total: u64,
        virtual_available: u64,
        extended: u64,
    }
    #[repr(C)]
    struct Value {
        status: u32,
        value: f64,
    }
    #[repr(C)]
    struct Item {
        name: *const u16,
        value: Value,
    }
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetSystemTimes(idle: *mut FileTime, kernel: *mut FileTime, user: *mut FileTime) -> i32;
        fn GlobalMemoryStatusEx(memory: *mut Memory) -> i32;
    }
    #[link(name = "pdh")]
    unsafe extern "system" {
        fn PdhOpenQueryW(source: *const u16, data: usize, query: *mut isize) -> u32;
        fn PdhAddEnglishCounterW(
            query: isize,
            path: *const u16,
            data: usize,
            counter: *mut isize,
        ) -> u32;
        fn PdhCollectQueryData(query: isize) -> u32;
        fn PdhGetFormattedCounterArrayW(
            counter: isize,
            format: u32,
            bytes: *mut u32,
            count: *mut u32,
            items: *mut c_void,
        ) -> u32;
        fn PdhCloseQuery(query: isize) -> u32;
    }
    pub(super) struct Platform {
        query: isize,
        counter: isize,
    }
    impl Default for Platform {
        fn default() -> Self {
            unsafe {
                let mut p = Self {
                    query: 0,
                    counter: 0,
                };
                if PdhOpenQueryW(ptr::null(), 0, &mut p.query) == 0 {
                    let path: Vec<u16> = "\\GPU Engine(*)\\Utilization Percentage"
                        .encode_utf16()
                        .chain(Some(0))
                        .collect();
                    if PdhAddEnglishCounterW(p.query, path.as_ptr(), 0, &mut p.counter) != 0 {
                        p.counter = 0;
                    }
                    PdhCollectQueryData(p.query);
                }
                p
            }
        }
    }
    impl Drop for Platform {
        fn drop(&mut self) {
            if self.query != 0 {
                unsafe {
                    PdhCloseQuery(self.query);
                }
            }
        }
    }
    impl Platform {
        pub(super) fn read(&mut self) -> (Option<(u64, u64)>, Option<f32>, Vec<(String, u32)>) {
            unsafe {
                let (mut idle, mut kernel, mut user) = (
                    FileTime::default(),
                    FileTime::default(),
                    FileTime::default(),
                );
                let ticks = (GetSystemTimes(&mut idle, &mut kernel, &mut user) != 0)
                    .then(|| (kernel.ticks() + user.ticks(), idle.ticks()));
                let mut mem = Memory {
                    length: std::mem::size_of::<Memory>() as u32,
                    ..Default::default()
                };
                let ram = if GlobalMemoryStatusEx(&mut mem) != 0 && mem.total > 0 {
                    Some(100. * (mem.total - mem.available) as f32 / mem.total as f32)
                } else {
                    None
                };
                (ticks, ram, self.gpu())
            }
        }
        fn gpu(&mut self) -> Vec<(String, u32)> {
            unsafe {
                if self.counter == 0 || PdhCollectQueryData(self.query) != 0 {
                    return Vec::new();
                }
                let (mut bytes, mut count) = (0, 0);
                const DOUBLE: u32 = 0x200;
                const MORE: u32 = 0x800007d2;
                if PdhGetFormattedCounterArrayW(
                    self.counter,
                    DOUBLE,
                    &mut bytes,
                    &mut count,
                    ptr::null_mut(),
                ) != MORE
                    || bytes == 0
                    || bytes > 16 * 1024 * 1024
                {
                    return Vec::new();
                }
                // u64 allocation supplies alignment for the native item structures.
                let mut buffer = vec![0u64; (bytes as usize + 7) / 8];
                if PdhGetFormattedCounterArrayW(
                    self.counter,
                    DOUBLE,
                    &mut bytes,
                    &mut count,
                    buffer.as_mut_ptr().cast(),
                ) != 0
                    || count as usize > buffer.len() * 8 / std::mem::size_of::<Item>()
                {
                    return Vec::new();
                }
                let items =
                    std::slice::from_raw_parts(buffer.as_ptr().cast::<Item>(), count as usize);
                let mut engines: BTreeMap<String, f64> = BTreeMap::new();
                for item in items {
                    if item.value.status > 1 || !item.value.value.is_finite() || item.name.is_null()
                    {
                        continue;
                    }
                    let mut len = 0;
                    while len < 1024 && *item.name.add(len) != 0 {
                        len += 1;
                    }
                    let name = String::from_utf16_lossy(std::slice::from_raw_parts(item.name, len));
                    // Remove pid so different processes on the same hardware engine are summed.
                    if let Some((_, engine)) = name.split_once("_luid_") {
                        *engines.entry(engine.to_owned()).or_default() += item.value.value.max(0.);
                    }
                }
                let mut devices: BTreeMap<String, u32> = BTreeMap::new();
                for (engine, value) in engines {
                    let device = engine.split("_eng_").next().unwrap_or(&engine).to_owned();
                    let v = devices.entry(device).or_default();
                    *v = (*v).max(value.clamp(0., 100.).round() as u32);
                }
                devices.into_iter().collect()
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cpu_excludes_guest_double_count_and_includes_iowait() {
        assert_eq!(
            parse_cpu("cpu 10 20 30 40 5 6 7 8 999 999"),
            Some((126, 45))
        );
        assert_eq!(cpu_delta((100, 30), (200, 55)), Some(75.));
        assert_eq!(cpu_delta((100, 30), (100, 30)), None);
        assert_eq!(cpu_delta((100, 30), (50, 20)), None);
    }
    #[test]
    fn memory_counts_reclaimable_cache_as_available() {
        assert_eq!(
            parse_memory("MemTotal: 1000 kB\nMemFree: 10 kB\nMemAvailable: 400 kB"),
            Some(60.)
        );
        assert_eq!(parse_memory("MemTotal: 0 kB\nMemAvailable: 0 kB"), None);
    }
    #[test]
    fn sampler_stops_on_drop() {
        let mut r = Resources::default();
        r.sample();
        drop(r);
    }
}
