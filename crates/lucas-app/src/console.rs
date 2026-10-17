use std::fs::OpenOptions;
use std::os::windows::io::IntoRawHandle;
use windows_sys::Win32::Foundation::{GetHandleInformation, HANDLE, INVALID_HANDLE_VALUE};
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

pub fn attach_parent() {
    unsafe {
        let handles = [STD_INPUT_HANDLE, STD_OUTPUT_HANDLE, STD_ERROR_HANDLE].map(|stream| {
            let handle = GetStdHandle(stream);
            (stream, handle, valid(handle))
        });
        if AttachConsole(ATTACH_PARENT_PROCESS) == 0 {
            return;
        }
        for (stream, handle, preserve) in handles {
            if preserve {
                SetStdHandle(stream, handle);
            } else if stream != STD_INPUT_HANDLE
                && !valid(GetStdHandle(stream))
                && let Ok(console) = OpenOptions::new().write(true).open("CONOUT$")
            {
                SetStdHandle(stream, console.into_raw_handle());
            }
        }
    }
}
