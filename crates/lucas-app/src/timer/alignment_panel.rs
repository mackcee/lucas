//! Focused native controls for alignment. Input is local window input only;
//! no global hotkeys, keyboard hooks, or messages/input sent to League.
use super::{alignment::Adjustment, native::wide};
use lucas_geometry::Rect;
use std::{
    io,
    ptr::{null, null_mut},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const CLASS: &str = "LUCAS.AlignmentPanel";

#[derive(Debug, PartialEq, Eq)]
pub enum KeyAction {
    Adjust(Adjustment),
    Save,
    Cancel,
}

fn command(id: usize) -> Option<KeyAction> {
    Some(match id {
        1 => KeyAction::Adjust(Adjustment::Left),
        2 => KeyAction::Adjust(Adjustment::Right),
        3 => KeyAction::Adjust(Adjustment::Up),
        4 => KeyAction::Adjust(Adjustment::Down),
        5 => KeyAction::Adjust(Adjustment::Shrink),
        6 => KeyAction::Adjust(Adjustment::Grow),
        9 => KeyAction::Save,
        10 => KeyAction::Cancel,
        _ => return None,
    })
}

fn key_action(key: u16, shift: bool) -> Option<KeyAction> {
    command(match (key, shift) {
        (VK_LEFT, false) => 1,
        (VK_RIGHT, false) => 2,
        (VK_UP, false) => 3,
        (VK_DOWN, false) => 4,
        (VK_LEFT | VK_DOWN, true) => 5,
        (VK_RIGHT | VK_UP, true) => 6,
        (VK_RETURN, _) => 9,
        (VK_ESCAPE, _) => 10,
        _ => return None,
    })
}

struct State {
    actions: Vec<KeyAction>,
}

pub struct Panel {
    window: HWND,
    font: HFONT,
    // Boxed for a stable callback address, borrowed only during dispatch.
    state: Box<State>,
}

impl Panel {
    pub fn new(target: HWND, map: Rect) -> io::Result<Self> {
        // SAFETY: all native resources belong to this UI thread. The boxed
        // queue stays at a stable address until user data is cleared in Drop.
        unsafe {
            let instance = GetModuleHandleW(null());
            let class = wide(CLASS);
            let definition = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                hCursor: LoadCursorW(null_mut(), IDC_ARROW),
                hbrBackground: (COLOR_BTNFACE + 1) as HBRUSH,
                ..Default::default()
            };
            if RegisterClassW(&definition) == 0 {
                return Err(io::Error::last_os_error());
            }
            let result = Self::create(target, map, instance, &class);
            if result.is_err() {
                UnregisterClassW(class.as_ptr(), instance);
            }
            result
        }
    }

    unsafe fn create(
        target: HWND,
        map: Rect,
        instance: HINSTANCE,
        class: &[u16],
    ) -> io::Result<Self> {
        // SAFETY: caller registered the class on this thread. Coordinates use
        // target-monitor DPI; monitor/work-area queries never activate League.
        unsafe {
            let dpi = GetDpiForWindow(target).max(96);
            let px = |value: i32| value * dpi as i32 / 96;
            let mut bounds = RECT {
                left: 0,
                top: 0,
                right: px(440),
                bottom: px(270),
            };
            let style = WS_CAPTION | WS_SYSMENU;
            // Keep the panel in Alt+Tab/taskbar so the draft remains reachable
            // after League or another application covers it.
            let ex_style = WS_EX_APPWINDOW | WS_EX_CONTROLPARENT;
            if AdjustWindowRectExForDpi(&mut bounds, style, 0, ex_style, dpi) == 0 {
                return Err(io::Error::last_os_error());
            }
            let mut monitor = MONITORINFO {
                cbSize: size_of::<MONITORINFO>() as u32,
                ..Default::default()
            };
            if GetMonitorInfoW(
                MonitorFromWindow(target, MONITOR_DEFAULTTONEAREST),
                &mut monitor,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
            let width = bounds.right - bounds.left;
            let height = bounds.bottom - bounds.top;
            let (x, y) = panel_position(monitor.rcWork, width, height, map);
            let window = CreateWindowExW(
                ex_style,
                class.as_ptr(),
                wide("LUCAS — Align minimap").as_ptr(),
                style,
                x,
                y,
                width,
                height,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if window.is_null() {
                return Err(io::Error::last_os_error());
            }
            let mut panel = Self {
                window,
                font: null_mut(),
                state: Box::new(State {
                    actions: Vec::new(),
                }),
            };
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                (&mut *panel.state as *mut State) as isize,
            );
            // Standard Windows controls provide native keyboard focus, system
            // colours and accessible names. Logical dimensions scale with DPI.
            panel.font = CreateFontW(
                -px(12),
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                DEFAULT_CHARSET as u32,
                OUT_DEFAULT_PRECIS as u32,
                CLIP_DEFAULT_PRECIS as u32,
                DEFAULT_QUALITY as u32,
                DEFAULT_PITCH as u32,
                wide("Segoe UI").as_ptr(),
            );
            if panel.font.is_null() {
                return Err(io::Error::last_os_error());
            }
            let controls = [
                (
                    "STATIC",
                    "Keep this panel focused; watch the ring over League.\nArrows move 1 px. Shift+arrows resize 1 px.\nEnter saves. Esc cancels.",
                    0,
                    16,
                    14,
                    408,
                    58,
                ),
                ("STATIC", "Move border", 0, 16, 82, 216, 20),
                ("BUTTON", "Up", 3, 88, 106, 68, 30),
                ("BUTTON", "Left", 1, 16, 140, 68, 30),
                ("BUTTON", "Down", 4, 88, 140, 68, 30),
                ("BUTTON", "Right", 2, 160, 140, 68, 30),
                ("STATIC", "Resize square", 0, 252, 82, 172, 20),
                ("BUTTON", "Smaller", 5, 252, 106, 82, 30),
                ("BUTTON", "Larger", 6, 342, 106, 82, 30),
                (
                    "STATIC",
                    "Bottom / outer corner stays anchored.\nSaved only when you choose Save.",
                    0,
                    16,
                    184,
                    408,
                    30,
                ),
                ("BUTTON", "Save", 9, 244, 226, 86, 30),
                ("BUTTON", "Cancel", 10, 338, 226, 86, 30),
            ];
            for (kind, label, id, x, y, width, height) in controls {
                let style = WS_CHILD
                    | WS_VISIBLE
                    | if id == 0 {
                        0
                    } else {
                        WS_TABSTOP | BS_PUSHBUTTON as u32
                    };
                let control = CreateWindowExW(
                    0,
                    wide(kind).as_ptr(),
                    wide(label).as_ptr(),
                    style,
                    px(x),
                    px(y),
                    px(width),
                    px(height),
                    window,
                    id as HMENU,
                    instance,
                    null(),
                );
                if control.is_null() {
                    return Err(io::Error::last_os_error());
                }
                SendMessageW(control, WM_SETFONT, panel.font as WPARAM, 1);
            }
            // Only this explicit tray action requests focus, and only for LUCAS.
            // The preview never activates; League is never brought foreground.
            ShowWindow(window, SW_SHOW);
            SetForegroundWindow(window);
            SetFocus(window);
            Ok(panel)
        }
    }

    pub fn handle(&self) -> HWND {
        self.window
    }

    pub fn take_actions(&mut self) -> Vec<KeyAction> {
        std::mem::take(&mut self.state.actions)
    }

    /// Called before Translate/Dispatch, including when a button has focus.
    pub fn process_message(&mut self, message: &MSG) -> bool {
        // SAFETY: live owned window and a message from this thread's queue.
        unsafe {
            if message.hwnd != self.window && IsChild(self.window, message.hwnd) == 0 {
                return false;
            }
            if message.message == WM_KEYDOWN
                && let Some(action) =
                    key_action(message.wParam as u16, GetKeyState(VK_SHIFT as i32) < 0)
            {
                self.state.actions.push(action);
                return true;
            }
            // Native Tab/Shift+Tab navigation and Space activation for buttons.
            IsDialogMessageW(self.window, message) != 0
        }
    }
}

impl Drop for Panel {
    fn drop(&mut self) {
        // SAFETY: destroy on the creating thread, with no outstanding borrow of
        // the queue. WM_CLOSE queues Cancel instead of destroying this handle.
        unsafe {
            SetWindowLongPtrW(self.window, GWLP_USERDATA, 0);
            DestroyWindow(self.window);
            if !self.font.is_null() {
                DeleteObject(self.font);
            }
            UnregisterClassW(wide(CLASS).as_ptr(), GetModuleHandleW(null()));
        }
    }
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    let action = match message {
        WM_CLOSE => Some(KeyAction::Cancel),
        WM_COMMAND if (w >> 16) == BN_CLICKED as usize && l != 0 => command(w & 0xffff),
        _ => None,
    };
    if let Some(action) = action {
        // SAFETY: user data is installed only while the boxed queue is live.
        unsafe {
            if let Some(state) = (GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State).as_mut() {
                state.actions.push(action);
            }
        }
        return 0;
    }
    // SAFETY: unchanged native callback arguments.
    unsafe { DefWindowProcW(window, message, w, l) }
}

fn panel_position(work: RECT, width: i32, height: i32, map: Rect) -> (i32, i32) {
    let centre_x = work.left + (work.right - work.left - width) / 2;
    let centre_y = work.top + (work.bottom - work.top - height) / 2;
    let candidates = [
        (centre_x, centre_y),
        (centre_x, work.top),
        (work.left, work.top),
        (work.right - width, work.top),
    ];
    candidates
        .into_iter()
        .find(|&(x, y)| {
            (x + width) as f64 <= map.left() - 16.0
                || x as f64 >= map.right() + 16.0
                || (y + height) as f64 <= map.top() - 16.0
                || y as f64 >= map.bottom() + 16.0
        })
        .unwrap_or((centre_x, work.top))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn local_keys_and_buttons_have_the_same_actions() {
        for (key, normal, shifted) in [
            (VK_LEFT, 1, 5),
            (VK_RIGHT, 2, 6),
            (VK_UP, 3, 6),
            (VK_DOWN, 4, 5),
            (VK_RETURN, 9, 9),
            (VK_ESCAPE, 10, 10),
        ] {
            assert_eq!(key_action(key, false), command(normal));
            assert_eq!(key_action(key, true), command(shifted));
        }
        assert_eq!(key_action(VK_SPACE, false), None);
        assert_eq!(command(0), None);
    }

    #[test]
    fn panel_avoids_minimap_on_either_side_and_negative_monitor_origin() {
        for left in [0, -3840] {
            for flip in [false, true] {
                let client = Rect::new(left as f64, 0.0, (left + 3840) as f64, 2160.0).unwrap();
                let map = lucas_geometry::minimap_rect(
                    client,
                    lucas_geometry::MinimapSettings {
                        minimap_scale: 2.1,
                        flipped: flip,
                    },
                    Default::default(),
                )
                .unwrap();
                let work = RECT {
                    left,
                    top: 0,
                    right: left + 3840,
                    bottom: 2160,
                };
                let (x, y) = panel_position(work, 880, 600, map);
                assert!(x >= work.left && x + 880 <= work.right);
                assert!(y >= work.top && y + 600 <= work.bottom);
                assert!(
                    (y + 600) as f64 <= map.top()
                        || (x + 880) as f64 <= map.left()
                        || x as f64 >= map.right()
                );
            }
        }
    }
}
