const RELEASES_URL: &str = "https://github.com/Ezreal-byte/lolping-lite/releases/latest";

/// Explicit settings-button action; never invoked automatically at startup.
#[cfg(target_os = "windows")]
pub fn open_releases() -> Result<(), String> {
    use std::ptr;
    use windows_sys::Win32::{UI::Shell::ShellExecuteW, UI::WindowsAndMessaging::SW_SHOWNORMAL};
    let url: Vec<u16> = RELEASES_URL.encode_utf16().chain(Some(0)).collect();
    let verb: Vec<u16> = "open".encode_utf16().chain(Some(0)).collect();
    let result = unsafe {
        ShellExecuteW(
            ptr::null_mut(),
            verb.as_ptr(),
            url.as_ptr(),
            ptr::null(),
            ptr::null(),
            SW_SHOWNORMAL,
        )
    } as isize;
    if result <= 32 {
        Err(format!("ShellExecuteW {result}"))
    } else {
        Ok(())
    }
}

#[cfg(target_os = "linux")]
pub fn open_releases() -> Result<(), String> {
    use std::process::{Command, Stdio};
    // The destination is fixed, never concatenated into a shell command.
    let status = Command::new("xdg-open")
        .arg(RELEASES_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|error| format!("Cannot open the download page: {error}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("xdg-open exited with {status}"))
    }
}

#[cfg(target_os = "macos")]
pub fn open_releases() -> Result<(), String> {
    use std::process::{Command, Stdio};
    let status = Command::new("/usr/bin/open")
        .arg(RELEASES_URL)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("open exited with {status}"))
    }
}
