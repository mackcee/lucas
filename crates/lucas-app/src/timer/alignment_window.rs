//! Static alignment preview only. Uses the existing overlay renderer and its
//! foreground/fullscreen gate, but holds one border frame instead of a pulse.
//! No hooks or input synthesis: temporary hotkeys exist only while aligning.
use super::{alignment::Adjustment, native::wide};
use lucas_geometry::Rect;
use lucas_overlay::{
    Pulse, PulseStyle, Salience,
    render::{self, Surface},
    win32::{TargetEligibility, TargetWindow},
};
use std::{
    io,
    ptr::{null, null_mut},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::LibraryLoader::GetModuleHandleW,
    UI::{Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const CLASS: &str = "LUCAS.Alignment";
const KEYS: [(u32, u32); 10] = [
    (0, VK_LEFT as u32),
    (0, VK_RIGHT as u32),
    (0, VK_UP as u32),
    (0, VK_DOWN as u32),
    (MOD_SHIFT, VK_LEFT as u32),
    (MOD_SHIFT, VK_RIGHT as u32),
    (MOD_SHIFT, VK_UP as u32),
    (MOD_SHIFT, VK_DOWN as u32),
    (MOD_NOREPEAT, VK_RETURN as u32),
    (MOD_NOREPEAT, VK_ESCAPE as u32),
];

pub enum KeyAction {
    Adjust(Adjustment),
    Save,
    Cancel,
}

pub fn action(id: usize) -> Option<KeyAction> {
    Some(match id {
        1 => KeyAction::Adjust(Adjustment::Left),
        2 => KeyAction::Adjust(Adjustment::Right),
        3 => KeyAction::Adjust(Adjustment::Up),
        4 => KeyAction::Adjust(Adjustment::Down),
        5 | 8 => KeyAction::Adjust(Adjustment::Shrink),
        6 | 7 => KeyAction::Adjust(Adjustment::Grow),
        9 => KeyAction::Save,
        10 => KeyAction::Cancel,
        _ => return None,
    })
}

struct State {
    target: usize,
    keys: bool,
}

pub struct Preview {
    window: HWND,
    state: Box<State>,
    rect: Option<Rect>,
}

impl Preview {
    pub fn new(target: HWND) -> io::Result<Self> {
        // SAFETY: this thread owns the registered class/window; state is boxed
        // and kept alive until after DestroyWindow. Window proc borrows it only
        // during dispatch, never concurrently with application mutations.
        unsafe {
            let instance = GetModuleHandleW(null());
            let class = wide(CLASS);
            let definition = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            if RegisterClassW(&definition) == 0 {
                return Err(io::Error::last_os_error());
            }
            let window = CreateWindowExW(
                WS_EX_LAYERED
                    | WS_EX_TRANSPARENT
                    | WS_EX_TOPMOST
                    | WS_EX_NOACTIVATE
                    | WS_EX_TOOLWINDOW,
                class.as_ptr(),
                wide("LUCAS align minimap").as_ptr(),
                WS_POPUP,
                0,
                0,
                1,
                1,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if window.is_null() {
                let error = io::Error::last_os_error();
                UnregisterClassW(class.as_ptr(), instance);
                return Err(error);
            }
            let mut preview = Self {
                window,
                state: Box::new(State {
                    target: target as usize,
                    keys: false,
                }),
                rect: None,
            };
            SetWindowLongPtrW(
                window,
                GWLP_USERDATA,
                (&mut *preview.state as *mut State) as isize,
            );
            if SetTimer(window, 1, 16, None) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(preview)
        }
    }

    pub fn handle(&self) -> HWND {
        self.window
    }

    pub fn hide(&mut self) {
        // SAFETY: window and hotkey registrations are owned on this thread.
        unsafe {
            suspend(self.window, &mut self.state);
        }
    }

    pub fn show(&mut self, rect: Rect) -> io::Result<()> {
        if TargetWindow::new(self.state.target as HWND)
            .check()
            .is_err()
        {
            self.hide();
            return Ok(());
        }
        // SAFETY: owned live window. Partial hotkey registration is rolled back.
        unsafe {
            if !self.state.keys {
                for (index, (modifiers, key)) in KEYS.iter().enumerate() {
                    if RegisterHotKey(self.window, index as i32 + 1, *modifiers, *key) == 0 {
                        let error = io::Error::last_os_error();
                        for id in 1..=index {
                            UnregisterHotKey(self.window, id as i32);
                        }
                        return Err(error);
                    }
                }
                self.state.keys = true;
            }
        }
        if self.rect != Some(rect) {
            present(self.window, rect)?;
            self.rect = Some(rect);
        }
        // Recheck after rendering, and never activate the game or the preview.
        // A foreground race suspends the draft, not an error that loses edits.
        if TargetWindow::new(self.state.target as HWND)
            .check()
            .is_err()
        {
            self.hide();
            return Ok(());
        }
        // SAFETY: the preview window is owned and live on this thread.
        unsafe {
            if SetWindowPos(
                self.window,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE | SWP_SHOWWINDOW,
            ) == 0
            {
                return Err(io::Error::last_os_error());
            }
        }
        Ok(())
    }
}

impl Drop for Preview {
    fn drop(&mut self) {
        self.hide();
        // SAFETY: no callbacks can borrow State after clearing user data and
        // destroying the window, all on its creating thread.
        unsafe {
            SetWindowLongPtrW(self.window, GWLP_USERDATA, 0);
            KillTimer(self.window, 1);
            DestroyWindow(self.window);
            UnregisterClassW(wide(CLASS).as_ptr(), GetModuleHandleW(null()));
        }
    }
}

unsafe fn suspend(window: HWND, state: &mut State) {
    // SAFETY: caller owns window and registrations on the dispatch thread.
    unsafe {
        ShowWindow(window, SW_HIDE);
        if state.keys {
            for id in 1..=10 {
                UnregisterHotKey(window, id);
            }
            state.keys = false;
        }
    }
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_TIMER => {
            // SAFETY: pointer is set after creation and cleared before destruction;
            // dispatch is on the owning thread. This also runs in tray menu loops.
            unsafe {
                let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State;
                if let Some(state) = state.as_mut()
                    && TargetWindow::new(state.target as HWND).check().is_err()
                {
                    suspend(window, state);
                }
            }
            0
        }
        WM_CLOSE | WM_DPICHANGED => 0,
        // SAFETY: unchanged window callback arguments.
        _ => unsafe { DefWindowProcW(window, message, w, l) },
    }
}

fn present(window: HWND, rect: Rect) -> io::Result<()> {
    let surface = Surface::for_target(rect).map_err(io::Error::other)?;
    let image = render::frame(
        surface,
        Pulse {
            style: PulseStyle::BreathingBorder,
            salience: Salience::Strong,
            ..Pulse::default()
        },
        Duration::from_millis(300),
    )
    .map_err(io::Error::other)?;
    // SAFETY: a bounded top-down DIB receives exactly width*height*4 BGRA bytes.
    // All GDI resources are released on both success and failure.
    unsafe {
        let dc = CreateCompatibleDC(null_mut());
        if dc.is_null() {
            return Err(io::Error::last_os_error());
        }
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: surface.width() as i32,
                biHeight: -(surface.height() as i32),
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = null_mut();
        let bitmap = CreateDIBSection(dc, &info, DIB_RGB_COLORS, &mut bits, null_mut(), 0);
        if bitmap.is_null() {
            let error = io::Error::last_os_error();
            DeleteDC(dc);
            return Err(error);
        }
        let previous = SelectObject(dc, bitmap);
        if previous.is_null() || previous as isize == GDI_ERROR as isize {
            let error = io::Error::last_os_error();
            DeleteObject(bitmap);
            DeleteDC(dc);
            return Err(error);
        }
        render::copy_bgra(
            &image,
            std::slice::from_raw_parts_mut(bits.cast(), image.data().len()),
        );
        let position = POINT {
            x: surface.left(),
            y: surface.top(),
        };
        let size = SIZE {
            cx: surface.width() as i32,
            cy: surface.height() as i32,
        };
        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as u8,
            BlendFlags: 0,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as u8,
        };
        let result = if UpdateLayeredWindow(
            window,
            null_mut(),
            &position,
            &size,
            dc,
            &POINT { x: 0, y: 0 },
            0,
            &blend,
            ULW_ALPHA,
        ) == 0
        {
            Err(io::Error::last_os_error())
        } else {
            Ok(())
        };
        GdiFlush();
        SelectObject(dc, previous);
        DeleteObject(bitmap);
        DeleteDC(dc);
        result
    }
}
