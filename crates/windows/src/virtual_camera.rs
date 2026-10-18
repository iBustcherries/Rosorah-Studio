//! Rosorah's Windows 11 virtual camera. The camera component (a Media Foundation source the
//! installer registers) reads frames from `PIPE`. Until it is installed, the camera stays off.

/// The pipe the camera component reads NV12 frames from.
pub const PIPE: &str = r"\\.\pipe\RosorahVirtualCamera";

/// Whether the camera component is registered on this computer.
pub fn installed() -> bool {
    false
}
