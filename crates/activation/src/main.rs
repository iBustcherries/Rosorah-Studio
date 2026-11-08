//! Stands in for Rosorah in browser/test_session.py. It opens a small window, prints "ready" once the
//! compositor has made it the active window, waits for a key press there (the test sends one with
//! wtype), then prints an activation token asked for exactly as Rosorah asks for one.

#[cfg(target_os = "linux")]
fn main() {
    match window::token_after_input() {
        Ok(token) => println!("{token}"),
        Err(e) => {
            eprintln!("No activation token: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("Activation tokens are a Wayland feature");
    std::process::exit(1);
}

#[cfg(target_os = "linux")]
mod window {
    use std::io::Write;
    use std::os::fd::AsFd;
    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::{
        wl_buffer::WlBuffer, wl_compositor::WlCompositor, wl_registry::WlRegistry, wl_shm,
        wl_shm::WlShm, wl_shm_pool::WlShmPool, wl_surface::WlSurface,
    };
    use wayland_client::{Connection, Dispatch, Proxy, QueueHandle, delegate_noop};
    use wayland_protocols::xdg::shell::client::{xdg_surface, xdg_toplevel, xdg_wm_base};

    #[derive(Default)]
    struct Window {
        configured: bool,
        active: bool,
    }

    pub fn token_after_input() -> Result<String, String> {
        let conn = Connection::connect_to_env().map_err(|e| e.to_string())?;
        let (globals, mut queue) =
            registry_queue_init::<Window>(&conn).map_err(|e| e.to_string())?;
        let qh = queue.handle();
        let compositor: WlCompositor = globals.bind(&qh, 1..=4, ()).map_err(|e| e.to_string())?;
        let shm: WlShm = globals.bind(&qh, 1..=1, ()).map_err(|e| e.to_string())?;
        let wm: xdg_wm_base::XdgWmBase = globals.bind(&qh, 1..=1, ()).map_err(|e| e.to_string())?;
        let surface = compositor.create_surface(&qh, ());
        let toplevel = wm.get_xdg_surface(&surface, &qh, ()).get_toplevel(&qh, ());
        toplevel.set_title("Rosorah activation test".into());
        toplevel.set_app_id(rosorah_activation::APP_ID.into());
        surface.commit();
        let mut activation = rosorah_activation::Activation::new(conn.clone(), surface.clone())
            .ok_or("the compositor has no xdg-activation")?;

        let mut window = Window::default();
        while !window.configured {
            queue
                .blocking_dispatch(&mut window)
                .map_err(|e| e.to_string())?;
        }
        // A see-through 64×64 picture, so the window is mapped.
        let (w, h) = (64, 64);
        let dir = std::env::var_os("XDG_RUNTIME_DIR").ok_or("XDG_RUNTIME_DIR is not set")?;
        let path =
            std::path::Path::new(&dir).join(format!("rosorah-activation-{}", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| e.to_string())?;
        let _ = std::fs::remove_file(&path);
        file.set_len(w * h * 4).map_err(|e| e.to_string())?;
        let pool = shm.create_pool(file.as_fd(), (w * h * 4) as i32, &qh, ());
        let buffer = pool.create_buffer(
            0,
            w as i32,
            h as i32,
            w as i32 * 4,
            wl_shm::Format::Argb8888,
            &qh,
            (),
        );
        surface.attach(Some(&buffer), 0, 0);
        surface.commit();

        let mut ready_serial = None;
        loop {
            queue
                .dispatch_pending(&mut window)
                .map_err(|e| e.to_string())?;
            activation.pump();
            if window.active && ready_serial.is_none() {
                ready_serial = Some(activation.serial());
                println!("ready");
                std::io::stdout().flush().map_err(|e| e.to_string())?;
            }
            if ready_serial
                .is_some_and(|s| activation.serial().is_some() && activation.serial() != s)
            {
                return activation
                    .token()
                    .ok_or_else(|| "the compositor gave no token".into());
            }
            conn.flush().map_err(|e| e.to_string())?;
            // Reading the connection fills both queues: the window's and the activation's.
            if let Some(guard) = queue.prepare_read() {
                {
                    let fd = guard.connection_fd();
                    let mut fds = [rustix::event::PollFd::new(
                        &fd,
                        rustix::event::PollFlags::IN,
                    )];
                    rustix::event::poll(&mut fds, None).map_err(|e| e.to_string())?;
                }
                guard.read().map_err(|e| e.to_string())?;
            }
        }
    }

    impl Dispatch<xdg_wm_base::XdgWmBase, ()> for Window {
        fn event(
            _: &mut Self,
            wm: &xdg_wm_base::XdgWmBase,
            event: xdg_wm_base::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_wm_base::Event::Ping { serial } = event {
                wm.pong(serial);
            }
        }
    }
    impl Dispatch<xdg_surface::XdgSurface, ()> for Window {
        fn event(
            window: &mut Self,
            surface: &xdg_surface::XdgSurface,
            event: xdg_surface::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_surface::Event::Configure { serial } = event {
                surface.ack_configure(serial);
                window.configured = true;
            }
        }
    }
    impl Dispatch<xdg_toplevel::XdgToplevel, ()> for Window {
        fn event(
            window: &mut Self,
            _: &xdg_toplevel::XdgToplevel,
            event: xdg_toplevel::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let xdg_toplevel::Event::Configure { states, .. } = event {
                let activated = xdg_toplevel::State::Activated as u32;
                window.active = states
                    .as_chunks::<4>()
                    .0
                    .iter()
                    .any(|s| u32::from_ne_bytes(*s) == activated);
            }
        }
    }
    impl Dispatch<WlRegistry, GlobalListContents> for Window {
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
    delegate_noop!(Window: WlCompositor);
    delegate_noop!(Window: ignore WlSurface);
    delegate_noop!(Window: ignore WlShm);
    delegate_noop!(Window: WlShmPool);
    delegate_noop!(Window: ignore WlBuffer);
}
