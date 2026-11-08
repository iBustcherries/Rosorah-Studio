//! Wayland only lets a window take focus with an activation token from the app the user is in,
//! asked for with the serial of the user's latest click or key press there. The browser helpers
//! are separate processes, so when the user opens one of their windows from Rosorah, Rosorah asks
//! the compositor for a token for its own window and hands it over; the helper passes it on when it
//! shows or raises its window. Elsewhere there is no token and windows come forward as usual.

/// The app id tokens are asked for; Rosorah's windows and its helpers' windows all use it.
pub const APP_ID: &str = "io.rosorah.Studio";

#[cfg(target_os = "linux")]
mod wayland;
#[cfg(target_os = "linux")]
pub use wayland::Activation;

#[cfg(target_os = "linux")]
thread_local! {
    static WINDOW: std::cell::RefCell<Option<Activation>> = const { std::cell::RefCell::new(None) };
}

/// Remembers Rosorah's main window from its `wl_display` and `wl_surface`, and starts following
/// its input serials. Call it, [`pump`] and [`token`] on the thread that runs the window.
///
/// # Safety
/// Both pointers must stay valid for as long as the program runs.
pub unsafe fn remember(display: *mut std::ffi::c_void, surface: *mut std::ffi::c_void) {
    #[cfg(target_os = "linux")]
    WINDOW.with_borrow_mut(|w| *w = unsafe { Activation::from_foreign(display, surface) });
    #[cfg(not(target_os = "linux"))]
    let _ = (display, surface);
}

/// Takes in the input events that arrived since last time. Call it whenever the window gets input,
/// so they don't pile up.
pub fn pump() {
    #[cfg(target_os = "linux")]
    WINDOW.with_borrow_mut(|w| w.as_mut().map(Activation::pump));
}

/// A token for the remembered window, or `None` off Wayland or if the compositor has no
/// activation support.
pub fn token() -> Option<String> {
    #[cfg(target_os = "linux")]
    return WINDOW.with_borrow_mut(|w| w.as_mut()?.token());
    #[cfg(not(target_os = "linux"))]
    None
}
