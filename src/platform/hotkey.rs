use anyhow::{bail, Result};

const MOD_ALT: u32 = 0x0001;
const MOD_SHIFT: u32 = 0x0004;
const MOD_NOREPEAT: u32 = 0x4000;
const VK_SPACE: u32 = 0x20;

#[link(name = "user32")]
extern "system" {
    fn RegisterHotKey(hwnd: *mut core::ffi::c_void, id: i32, fs_modifiers: u32, vk: u32) -> i32;
    fn UnregisterHotKey(hwnd: *mut core::ffi::c_void, id: i32) -> i32;
}

#[cfg(windows)]
pub fn start_hotkey_listener<F>(on_hotkey: F) -> Result<std::thread::JoinHandle<()>>
where
    F: Fn() + Send + 'static,
{
    use std::{mem::zeroed, ptr, sync::mpsc};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG, WM_HOTKEY};

    const HOTKEY_ID: i32 = 0x4641;
    let (ready_tx, ready_rx) = mpsc::sync_channel(1);
    let handle = std::thread::Builder::new()
        .name("fastaccess-hotkey".into())
        .spawn(move || unsafe {
            let registered = RegisterHotKey(
                ptr::null_mut(),
                HOTKEY_ID,
                MOD_ALT | MOD_SHIFT | MOD_NOREPEAT,
                VK_SPACE,
            );
            let _ = ready_tx.send(registered != 0);
            if registered == 0 {
                return;
            }

            let mut message: MSG = zeroed();
            while GetMessageW(&mut message, ptr::null_mut(), 0, 0) > 0 {
                if message.message == WM_HOTKEY && message.wParam == HOTKEY_ID as usize {
                    on_hotkey();
                }
            }
            UnregisterHotKey(ptr::null_mut(), HOTKEY_ID);
        })?;

    if !ready_rx.recv()? {
        bail!(
            "cannot register Alt+Shift+Space; it may already be used by another application"
        );
    }
    Ok(handle)
}

#[cfg(not(windows))]
pub fn start_hotkey_listener<F>(_on_hotkey: F) -> Result<std::thread::JoinHandle<()>>
where
    F: Fn() + Send + 'static,
{
    bail!("global hotkeys are supported only on Windows")
}

