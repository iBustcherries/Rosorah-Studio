pub struct RendererBackend;
impl rosorah_capture::Backend for RendererBackend {
    fn run(
        self: Box<Self>,
        output: rosorah_capture::Producer,
        cancel: tokio::sync::watch::Receiver<bool>,
        options: rosorah_capture::Options,
    ) -> anyhow::Result<()> {
        rosorah_capture::Backend::run(
            Box::new(rosorah_windows::window::WindowBackend { window: None }),
            output,
            cancel,
            options,
        )
    }
}
pub fn installation_status() -> String {
    "Windows monitor/window capture; automatic fullscreen detection uses Windows Graphics Capture. Injected renderer hooks are not included.".into()
}
