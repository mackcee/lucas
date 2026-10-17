use std::io;

/// Physical screen coordinates; the host executable must be per-monitor DPI aware.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ScreenRect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DisplayMode {
    Windowed,
    Borderless,
    /// Best-effort detection, not a guarantee about the game's swapchain.
    ExclusiveFullscreen,
    #[default]
    Unknown,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct WindowStatus {
    pub exists: bool,
    pub foreground: bool,
    pub minimized: bool,
    pub client_rect: Option<ScreenRect>,
    pub display_mode: DisplayMode,
}

impl WindowStatus {
    /// Also usable for an explicitly selected window-only fallback.
    pub fn overlay_eligible(self) -> bool {
        self.exists
            && self.foreground
            && !self.minimized
            && self.client_rect.is_some()
            && matches!(
                self.display_mode,
                DisplayMode::Windowed | DisplayMode::Borderless
            )
    }
}

pub trait WindowProbe {
    fn sample(&mut self) -> io::Result<WindowStatus>;
}

/// Set `status` between samples to drive portable integration tests.
#[derive(Debug, Default)]
pub struct FakeWindowProbe {
    pub status: WindowStatus,
}

impl WindowProbe for FakeWindowProbe {
    fn sample(&mut self) -> io::Result<WindowStatus> {
        Ok(self.status)
    }
}

#[cfg(windows)]
pub use win32::Win32WindowProbe;

#[cfg(any(windows, test))]
fn classify_display_mode(
    covers_monitor: bool,
    decorated: bool,
    foreground: bool,
    shell_fullscreen: Option<bool>,
) -> DisplayMode {
    if !covers_monitor || decorated {
        DisplayMode::Windowed
    } else {
        match shell_fullscreen {
            None => DisplayMode::Unknown,
            Some(true) if foreground => DisplayMode::ExclusiveFullscreen,
            Some(_) => DisplayMode::Borderless,
        }
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod win32 {
    use super::*;
    use std::{mem::size_of, ptr};
    use windows_sys::Win32::{
        Foundation::{POINT, RECT},
        Graphics::Gdi::{
            ClientToScreen, GetMonitorInfoW, MONITOR_DEFAULTTONEAREST, MONITORINFO,
            MonitorFromWindow,
        },
        UI::{
            Shell::{QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState},
            WindowsAndMessaging::{
                FindWindowW, GWL_STYLE, GetClientRect, GetForegroundWindow, GetWindowLongW,
                GetWindowRect, IsIconic, IsWindowVisible, WS_CAPTION, WS_THICKFRAME,
            },
        },
    };

    #[derive(Debug, Default)]
    pub struct Win32WindowProbe;

    impl WindowProbe for Win32WindowProbe {
        fn sample(&mut self) -> io::Result<WindowStatus> {
            let class: Vec<u16> = "RiotWindowClass\0".encode_utf16().collect();
            // SAFETY: class is a live NUL-terminated UTF-16 string. All output
            // pointers below refer to initialized, correctly sized local structs.
            // Window handles are borrowed only within this poll; every fallible
            // geometry call is checked (a disappearing window returns an error).
            unsafe {
                let hwnd = FindWindowW(class.as_ptr(), ptr::null());
                if hwnd.is_null() {
                    return Ok(WindowStatus::default());
                }
                let foreground = GetForegroundWindow() == hwnd;
                let minimized = IsIconic(hwnd) != 0 || IsWindowVisible(hwnd) == 0;
                if minimized {
                    return Ok(WindowStatus {
                        exists: true,
                        foreground,
                        minimized,
                        ..WindowStatus::default()
                    });
                }
                let mut client: RECT = std::mem::zeroed();
                let mut origin = POINT { x: 0, y: 0 };
                let mut window: RECT = std::mem::zeroed();
                if GetClientRect(hwnd, &mut client) == 0
                    || ClientToScreen(hwnd, &mut origin) == 0
                    || GetWindowRect(hwnd, &mut window) == 0
                {
                    return Err(io::Error::last_os_error());
                }
                let mut monitor: MONITORINFO = std::mem::zeroed();
                monitor.cbSize = size_of::<MONITORINFO>() as u32;
                let monitor_handle = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                if GetMonitorInfoW(monitor_handle, &mut monitor) == 0 {
                    return Err(io::Error::last_os_error());
                }
                let covers_monitor = window.left <= monitor.rcMonitor.left
                    && window.top <= monitor.rcMonitor.top
                    && window.right >= monitor.rcMonitor.right
                    && window.bottom >= monitor.rcMonitor.bottom;
                let style = GetWindowLongW(hwnd, GWL_STYLE) as u32;
                let decorated = style & (WS_CAPTION | WS_THICKFRAME) != 0;
                let mut notification_state = 0;
                let query_ok = SHQueryUserNotificationState(&mut notification_state) >= 0;
                let display_mode = classify_display_mode(
                    covers_monitor,
                    decorated,
                    foreground,
                    query_ok.then_some(notification_state == QUNS_RUNNING_D3D_FULL_SCREEN),
                );
                let rect = ScreenRect {
                    left: origin.x,
                    top: origin.y,
                    right: origin.x + client.right - client.left,
                    bottom: origin.y + client.bottom - client.top,
                };
                Ok(WindowStatus {
                    exists: true,
                    foreground,
                    minimized,
                    client_rect: (rect.right > rect.left && rect.bottom > rect.top).then_some(rect),
                    display_mode,
                })
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fullscreen_requires_all_three_signals() {
        for covers in [false, true] {
            for decorated in [false, true] {
                for foreground in [false, true] {
                    for shell in [None, Some(false), Some(true)] {
                        let result = classify_display_mode(covers, decorated, foreground, shell);
                        assert_eq!(
                            result == DisplayMode::ExclusiveFullscreen,
                            covers && !decorated && foreground && shell == Some(true)
                        );
                        assert_eq!(
                            result == DisplayMode::Unknown,
                            covers && !decorated && shell.is_none()
                        );
                        assert_eq!(result == DisplayMode::Windowed, !covers || decorated);
                    }
                }
            }
        }
    }
}
