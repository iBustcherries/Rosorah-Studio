//! System-wide hotkeys: registered with Windows so they work in any app, on a thread with its own
//! message loop. Key releases (for push-to-talk) come from the key's state, checked every 20 ms.
use std::sync::atomic::{AtomicBool, Ordering};
use windows::Win32::UI::{
    Input::KeyboardAndMouse::{GetAsyncKeyState, HOT_KEY_MODIFIERS, MOD_ALT, MOD_CONTROL, MOD_NOREPEAT, MOD_SHIFT, RegisterHotKey, UnregisterHotKey},
    WindowsAndMessaging::{MSG, PM_REMOVE, PeekMessageW, WM_HOTKEY},
};

/// `keys` are (virtual key, ctrl, alt, shift). Calls `event(index, down)` as keys go down and up,
/// and `taken(index)` for each combination Windows accepted. Runs until `stop` is set.
pub fn run(keys: &[(u32, bool, bool, bool)], stop: &AtomicBool, mut event: impl FnMut(usize, bool), mut taken: impl FnMut(usize)) -> anyhow::Result<()> {
    let mut held = vec![false; keys.len()];
    let mut accepted = vec![false; keys.len()];
    for (i, &(vk, ctrl, alt, shift)) in keys.iter().enumerate() {
        if vk == 0 {
            continue;
        }
        let mut modifiers = MOD_NOREPEAT;
        if ctrl {
            modifiers |= MOD_CONTROL;
        }
        if alt {
            modifiers |= MOD_ALT;
        }
        if shift {
            modifiers |= MOD_SHIFT;
        }
        // Registered to this thread (no window); another app may already own the combination.
        if unsafe { RegisterHotKey(None, i as i32 + 1, HOT_KEY_MODIFIERS(modifiers.0), vk) }.is_ok() {
            accepted[i] = true;
            taken(i);
        }
    }
    while !stop.load(Ordering::Relaxed) {
        let mut msg = MSG::default();
        while unsafe { PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE) }.as_bool() {
            if msg.message == WM_HOTKEY {
                let i = msg.wParam.0.saturating_sub(1);
                if i < keys.len() && !held[i] {
                    held[i] = true;
                    event(i, true);
                }
            }
        }
        // A held combination is released when its key comes up.
        for (i, &(vk, ..)) in keys.iter().enumerate() {
            if held[i] && (unsafe { GetAsyncKeyState(vk as i32) } as u16 & 0x8000) == 0 {
                held[i] = false;
                event(i, false);
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    for (i, ok) in accepted.iter().enumerate() {
        if *ok {
            let _ = unsafe { UnregisterHotKey(None, i as i32 + 1) };
        }
    }
    Ok(())
}
