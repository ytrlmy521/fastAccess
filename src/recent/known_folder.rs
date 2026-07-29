use anyhow::{bail, Result};
use std::path::PathBuf;

#[cfg(windows)]
pub fn recent_folder() -> Result<PathBuf> {
    use std::ffi::OsString;
    use std::os::windows::ffi::OsStringExt;
    use std::ptr;
    use windows_sys::Win32::System::Com::CoTaskMemFree;
    use windows_sys::Win32::UI::Shell::{FOLDERID_Recent, SHGetKnownFolderPath};

    let mut raw_path = ptr::null_mut();
    let result = unsafe {
        SHGetKnownFolderPath(
            &FOLDERID_Recent,
            0,
            ptr::null_mut(),
            &mut raw_path,
        )
    };

    if result < 0 {
        bail!("SHGetKnownFolderPath(FOLDERID_Recent) failed: HRESULT 0x{result:08X}");
    }

    if raw_path.is_null() {
        bail!("SHGetKnownFolderPath returned a null path");
    }

    let len = unsafe {
        let mut len = 0usize;
        while *raw_path.add(len) != 0 {
            len += 1;
        }
        len
    };
    let path = PathBuf::from(OsString::from_wide(unsafe {
        std::slice::from_raw_parts(raw_path, len)
    }));
    unsafe { CoTaskMemFree(raw_path.cast()) };
    Ok(path)
}

#[cfg(not(windows))]
pub fn recent_folder() -> Result<PathBuf> {
    bail!("Windows Recent Items is available only on Windows")
}

