//! Steady, click-through alignment border, paired with focused native controls.
//! Both League and the panel may be foreground; other eligibility gates remain.
//! No global hotkeys, hooks or input synthesis.
use super::{
    alignment::preview_eligible,
    alignment_panel::{KeyAction, Panel},
    native::wide,
};
use lucas_game::{Win32WindowProbe, WindowProbe};
use lucas_geometry::Rect;
use lucas_overlay::{
    Pulse, PulseStyle, Salience,
    render::{self, Surface},
};
use std::{
    io,
    ptr::{null, null_mut},
    time::Duration,
};
use windows_sys::Win32::{
    Foundation::*, Graphics::Gdi::*, System::LibraryLoader::GetModuleHandleW,
    UI::WindowsAndMessaging::*,
};

const CLASS: &str = "LUCAS.Alignment";
struct State {
    target: usize,
    panel: usize,
}

impl State {
    fn eligible(&self) -> bool {
        // SAFETY: queried handles only. A replaced game cannot inherit a draft.
        unsafe {
            FindWindowW(wide("RiotWindowClass").as_ptr(), null()) == self.target as HWND
                && preview_eligible(
                    Win32WindowProbe.sample().unwrap_or_default(),
                    GetForegroundWindow() == self.panel as HWND,
                )
        }
    }
}

pub struct Preview {
    window: HWND,
    state: Box<State>,
    panel: Panel,
    rect: Option<Rect>,
}

impl Preview {
    pub fn new(target: HWND, rect: Rect) -> io::Result<Self> {
        let panel = Panel::new(target, rect)?;
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
                    panel: panel.handle() as usize,
                }),
                panel,
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

    pub fn target(&self) -> HWND {
        self.state.target as HWND
    }

    pub fn eligible(&self) -> bool {
        self.state.eligible()
    }

    pub fn process_message(&mut self, message: &MSG) -> bool {
        self.panel.process_message(message)
    }

    pub fn take_actions(&mut self) -> Vec<KeyAction> {
        self.panel.take_actions()
    }

    pub fn hide(&mut self) {
        // SAFETY: owned border only; the control panel remains reachable.
        unsafe {
            ShowWindow(self.window, SW_HIDE);
        }
    }

    pub fn show(&mut self, rect: Rect) -> io::Result<()> {
        if !self.eligible() {
            self.hide();
            return Ok(());
        }
        if self.rect != Some(rect) {
            present(self.window, rect)?;
            self.rect = Some(rect);
        }
        // Recheck after rendering, and never activate the game or the preview.
        // A foreground race suspends the draft, not an error that loses edits.
        if !self.eligible() {
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

unsafe extern "system" fn window_proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        WM_TIMER => {
            // SAFETY: pointer is set after creation and cleared before destruction;
            // dispatch is on the owning thread. This also runs in tray menu loops.
            unsafe {
                let state = GetWindowLongPtrW(window, GWLP_USERDATA) as *mut State;
                if let Some(state) = state.as_ref()
                    && !state.eligible()
                {
                    ShowWindow(window, SW_HIDE);
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
