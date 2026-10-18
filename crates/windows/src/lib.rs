//! Native Windows services. No Linux portals, command-line helpers or renderer injection.
#[cfg(windows)]
pub mod capture;
#[cfg(windows)]
pub mod credentials;
pub mod pixels;

#[cfg(windows)]
pub mod audio;

#[cfg(windows)]
pub mod shell;

#[cfg(windows)]
pub mod pipe;

#[cfg(windows)]
pub mod window;

#[cfg(windows)]
pub mod hotkeys;

#[cfg(windows)]
pub mod virtual_camera;

pub mod capture_policy;
