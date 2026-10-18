pub const CHANNEL: u64 = 10;
pub fn alive(game: &rosorah_capture::GameProcess) -> bool {
    rosorah_windows::window::alive(game)
}
// Windows process loopback includes children itself. One root avoids duplicate audio.
pub fn family(game: &rosorah_capture::GameProcess) -> std::collections::BTreeSet<u32> {
    if alive(game) {
        [game.pid].into_iter().collect()
    } else {
        Default::default()
    }
}
