//! Wayland only lets a window take focus with an activation token from the app the user is in.
//! The browser helpers are separate processes, so when the user opens one of their windows from
//! Rosorah, Rosorah asks the compositor for a token for its own window and hands it over; the
//! helper passes it on when it shows or raises its window. Elsewhere there is no token and windows
//! come forward as usual.

use std::sync::OnceLock;

/// The app id tokens are asked for; Rosorah's windows and its helpers' windows all use it.
pub const APP_ID: &str = "io.rosorah.Studio";

static WINDOW: OnceLock<(usize, usize)> = OnceLock::new();

/// Remembers Rosorah's main window: its `wl_display` and `wl_surface`. They must stay valid for
/// as long as the program runs.
pub fn remember(display: *mut std::ffi::c_void, surface: *mut std::ffi::c_void) {
    let _ = WINDOW.set((display as usize, surface as usize));
}

/// A token for the remembered window, or `None` off Wayland or if the compositor has no
/// activation support.
pub fn token() -> Option<String> {
    let &(display, surface) = WINDOW.get()?;
    imp::for_window(display as _, surface as _)
}

/// A token from a connection of its own, with no window behind it. Compositors that check where
/// a token came from won't honour it; it's for tests.
pub fn standalone_token() -> Option<String> {
    imp::standalone()
}

#[cfg(target_os = "linux")]
mod imp {
    use wayland_client::backend::{Backend, ObjectId};
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{wl_registry::WlRegistry, wl_surface::WlSurface};
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle};
    use wayland_protocols::xdg::activation::v1::client::xdg_activation_token_v1::{
        self, XdgActivationTokenV1,
    };
    use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;

    #[derive(Default)]
    struct State(Option<String>);
    impl Dispatch<WlRegistry, GlobalListContents> for State {
        fn event(
            _: &mut Self,
            _: &WlRegistry,
            _: <WlRegistry as Proxy>::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    impl Dispatch<XdgActivationV1, ()> for State {
        fn event(
            _: &mut Self,
            _: &XdgActivationV1,
            _: <XdgActivationV1 as Proxy>::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }
    impl Dispatch<XdgActivationTokenV1, ()> for State {
        fn event(
            state: &mut Self,
            _: &XdgActivationTokenV1,
            event: xdg_activation_token_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_activation_token_v1::Event::Done { token } = event {
                state.0 = Some(token);
            }
        }
    }

    /// Asks on a queue of our own, so winit's events on the same connection are left alone.
    fn request(conn: &Connection, surface: Option<&WlSurface>) -> Option<String> {
        let (globals, mut queue) = registry_queue_init::<State>(conn).ok()?;
        let activation: XdgActivationV1 = globals.bind(&queue.handle(), 1..=1, ()).ok()?;
        let token = activation.get_activation_token(&queue.handle(), ());
        token.set_app_id(super::APP_ID.into());
        if let Some(surface) = surface {
            token.set_surface(surface);
        }
        token.commit();
        let mut state = State::default();
        // The compositor answers commit with done straight away; two round trips is plenty.
        for _ in 0..2 {
            if state.0.is_some() || queue.roundtrip(&mut state).is_err() {
                break;
            }
        }
        token.destroy();
        activation.destroy();
        state.0
    }

    pub fn for_window(
        display: *mut std::ffi::c_void,
        surface: *mut std::ffi::c_void,
    ) -> Option<String> {
        // Safety: remember()'s caller keeps both alive; a foreign display isn't closed on drop.
        let conn =
            Connection::from_backend(unsafe { Backend::from_foreign_display(display.cast()) });
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }.ok()?;
        let surface = WlSurface::from_id(&conn, id).ok()?;
        request(&conn, Some(&surface))
    }

    pub fn standalone() -> Option<String> {
        request(&Connection::connect_to_env().ok()?, None)
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    pub fn for_window(_: *mut std::ffi::c_void, _: *mut std::ffi::c_void) -> Option<String> {
        None
    }
    pub fn standalone() -> Option<String> {
        None
    }
}
