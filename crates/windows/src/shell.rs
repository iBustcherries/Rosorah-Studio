//! Open URLs and local files without passing them through cmd.exe.
use std::{ffi::OsStr, os::windows::ffi::OsStrExt};
use windows::{
    Win32::{
        Foundation::HWND,
        UI::{Shell::ShellExecuteW, WindowsAndMessaging::SW_SHOWNORMAL},
    },
    core::{PCWSTR, w},
};
pub fn open(target: &OsStr) -> anyhow::Result<()> {
    let mut wide: Vec<u16> = target.encode_wide().collect();
    anyhow::ensure!(
        !wide.is_empty() && !wide.contains(&0),
        "Invalid path or URL"
    );
    wide.push(0);
    let result = unsafe {
        ShellExecuteW(
            HWND::default(),
            w!("open"),
            PCWSTR(wide.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        )
    };
    anyhow::ensure!(
        result.0 as isize > 32,
        "Windows could not open the selected path or URL (code {})",
        result.0 as isize
    );
    Ok(())
}
