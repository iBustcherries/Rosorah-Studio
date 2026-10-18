//! Linux launch activation is deliberately not invoked on Windows.
pub struct Guard;
impl Guard {
    pub fn start() -> Self {
        Self
    }
}
pub fn status() -> String {
    "Automatic fullscreen capture is available through Windows Graphics Capture.".into()
}
pub fn needs_permission() -> bool {
    false
}
pub fn authorize() {}
