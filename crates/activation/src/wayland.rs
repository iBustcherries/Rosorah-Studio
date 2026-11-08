use wayland_client::backend::{Backend, ObjectId};
use wayland_client::globals::{GlobalListContents, registry_queue_init};
use wayland_client::protocol::wl_keyboard::{self, WlKeyboard};
use wayland_client::protocol::wl_pointer::{self, WlPointer};
use wayland_client::protocol::wl_seat::{self, WlSeat};
use wayland_client::protocol::{wl_registry::WlRegistry, wl_surface::WlSurface};
use wayland_client::{Connection, Dispatch, EventQueue, Proxy, QueueHandle, WEnum};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_token_v1::{
    self, XdgActivationTokenV1,
};
use wayland_protocols::xdg::activation::v1::client::xdg_activation_v1::XdgActivationV1;

/// Asks for activation tokens for one window, with the serial of its latest input.
///
/// It keeps a seat, pointer and keyboard of its own on a queue of its own. The compositor sends
/// those the same input events as the window's toolkit gets, which is how the serial of the click
/// that asked for a window is known; the toolkit's own events are left alone.
pub struct Activation {
    queue: EventQueue<State>,
    state: State,
    activation: XdgActivationV1,
    surface: WlSurface,
    _conn: Connection,
}

#[derive(Default)]
struct State {
    seat: Option<WlSeat>,
    pointer: Option<WlPointer>,
    keyboard: Option<WlKeyboard>,
    serial: Option<u32>,
    token: Option<String>,
}

impl Activation {
    pub fn new(conn: Connection, surface: WlSurface) -> Option<Self> {
        let (globals, mut queue) = registry_queue_init::<State>(&conn).ok()?;
        let activation = globals.bind(&queue.handle(), 1..=1, ()).ok()?;
        let mut state = State {
            seat: globals.bind(&queue.handle(), 1..=7, ()).ok(),
            ..State::default()
        };
        // The seat says what it has; that brings the pointer and keyboard.
        queue.roundtrip(&mut state).ok()?;
        Some(Self {
            queue,
            state,
            activation,
            surface,
            _conn: conn,
        })
    }

    /// For a window a toolkit made, on the toolkit's connection.
    ///
    /// # Safety
    /// Both pointers must stay valid while this lives; a foreign display isn't closed on drop.
    pub unsafe fn from_foreign(
        display: *mut std::ffi::c_void,
        surface: *mut std::ffi::c_void,
    ) -> Option<Self> {
        let conn =
            Connection::from_backend(unsafe { Backend::from_foreign_display(display.cast()) });
        let id = unsafe { ObjectId::from_ptr(WlSurface::interface(), surface.cast()) }.ok()?;
        let surface = WlSurface::from_id(&conn, id).ok()?;
        Self::new(conn, surface)
    }

    /// Handles input events the toolkit already read from the connection.
    pub fn pump(&mut self) {
        let _ = self.queue.dispatch_pending(&mut self.state);
    }

    /// The serial of the window's latest input, if it had any.
    pub fn serial(&self) -> Option<u32> {
        self.state.serial
    }

    pub fn token(&mut self) -> Option<String> {
        self.pump();
        let token = self
            .activation
            .get_activation_token(&self.queue.handle(), ());
        token.set_app_id(super::APP_ID.into());
        token.set_surface(&self.surface);
        if let (Some(serial), Some(seat)) = (self.state.serial, &self.state.seat) {
            token.set_serial(serial, seat);
        }
        token.commit();
        self.state.token = None;
        // The compositor answers commit with done straight away; two round trips is plenty.
        for _ in 0..2 {
            if self.state.token.is_some() || self.queue.roundtrip(&mut self.state).is_err() {
                break;
            }
        }
        token.destroy();
        self.state.token.take()
    }
}

impl Dispatch<WlSeat, ()> for State {
    fn event(
        state: &mut Self,
        seat: &WlSeat,
        event: wl_seat::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_seat::Event::Capabilities {
            capabilities: WEnum::Value(caps),
        } = event
        {
            if caps.contains(wl_seat::Capability::Pointer) && state.pointer.is_none() {
                state.pointer = Some(seat.get_pointer(qh, ()));
            }
            if caps.contains(wl_seat::Capability::Keyboard) && state.keyboard.is_none() {
                state.keyboard = Some(seat.get_keyboard(qh, ()));
            }
        }
    }
}
impl Dispatch<WlPointer, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_pointer::Event::Enter { serial, .. } | wl_pointer::Event::Button { serial, .. } => {
                state.serial = Some(serial)
            }
            _ => {}
        }
    }
}
impl Dispatch<WlKeyboard, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            wl_keyboard::Event::Enter { serial, .. } | wl_keyboard::Event::Key { serial, .. } => {
                state.serial = Some(serial)
            }
            _ => {}
        }
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
            state.token = Some(token);
        }
    }
}
wayland_client::delegate_noop!(State: XdgActivationV1);

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
