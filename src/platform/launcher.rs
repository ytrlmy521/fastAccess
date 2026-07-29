use anyhow::Result;
#[cfg(windows)]
use anyhow::bail;
#[cfg(not(windows))]
use anyhow::Context;
use std::path::Path;

#[cfg(windows)]
pub fn open_target(path: &Path) -> Result<()> {
    use std::{os::windows::ffi::OsStrExt, ptr};
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    let path: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            verb.as_ptr(),
            path.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        bail!("ShellExecuteW failed with code {}", result as isize);
    }
    Ok(())
}

#[cfg(not(windows))]
pub fn open_target(_path: &Path) -> Result<()> {
    Err(anyhow::anyhow!("opening targets is supported only on Windows"))
        .context("platform operation unavailable")
}
