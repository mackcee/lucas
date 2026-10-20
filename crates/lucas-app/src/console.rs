use std::fs::OpenOptions;
use std::os::windows::io::IntoRawHandle;
use windows_sys::Win32::Foundation::{GetHandleInformation, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::Storage::FileSystem::{FILE_TYPE_DISK, FILE_TYPE_PIPE, GetFileType};
use windows_sys::Win32::System::Console::{
    ATTACH_PARENT_PROCESS, AttachConsole, GetStdHandle, STD_ERROR_HANDLE, STD_INPUT_HANDLE,
    STD_OUTPUT_HANDLE, SetStdHandle,
};

fn valid(handle: HANDLE) -> bool {
    let mut flags = 0;
    !handle.is_null()
        && handle != INVALID_HANDLE_VALUE
        && unsafe { GetHandleInformation(handle, &mut flags) != 0 }
}

fn redirected(handle: HANDLE) -> bool {
    // A GUI process can inherit valid NUL character handles from PowerShell.
    // They are not output redirection: preserving them silently discards output.
    valid(handle)
        && matches!(
            unsafe { GetFileType(handle) },
            FILE_TYPE_DISK | FILE_TYPE_PIPE
        )
}
pub fn attach_parent() {
    unsafe {
        let handles = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].map(|stream| {
            let handle = GetStdHandle(stream);
            (stream, handle, redirected(handle))
        });
        // Failure may mean already attached; opening CONOUT$ handles that too.
        // No console is allocated for GUI-only/default tray launches.
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
        for (stream, handle, preserve) in handles {
            if preserve {
                SetStdHandle(stream, handle);
            } else if stream != STD_INPUT_HANDLE
                && let Ok(console) = OpenOptions::new().write(true).open("CONOUT$")
            {
                SetStdHandle(stream, console.into_raw_handle());
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;
    #[test]
    fn preserve_files_but_not_valid_nul_handles() {
        let null = OpenOptions::new().write(true).open("NUL").unwrap();
        assert!(valid(null.as_raw_handle()));
        assert!(!redirected(null.as_raw_handle()));
        let file = tempfile::tempfile().unwrap();
        assert!(redirected(file.as_raw_handle()));
    }
}
