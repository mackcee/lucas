//! Windows 10/11 layered window on a dedicated per-monitor-v2-aware thread.
//! No hooks, input synthesis, capture, or process access exist in this backend.

use crate::{
    Overlay, OverlayError, Pulse, Rect, Result,
    render::{self, Surface},
};
use std::{
    ptr::{null, null_mut},
    sync::{
        Arc,
        mpsc::{self, Receiver, SyncSender},
    },
    thread::{self, JoinHandle},
    time::Instant,
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::*,
    System::{
        LibraryLoader::GetModuleHandleW,
        Threading::{CreateEventW, GetCurrentThreadId, INFINITE, SetEvent},
    },
    UI::{
        HiDpi::*,
        Shell::{QUNS_RUNNING_D3D_FULL_SCREEN, SHQueryUserNotificationState},
        WindowsAndMessaging::*,
    },
};

const TIMER: usize = 1;
const EX_STYLE: u32 =
    WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;

/// Capture exclusion requires Windows 10 2004+ for full exclusion semantics.
/// Older Windows may provide WDA_MONITOR semantics instead. Never silently retry
/// without the requested affinity if the API fails.
#[derive(Debug, Clone, Copy, Default)]
pub struct Options {
    pub exclude_from_capture: bool,
}

pub trait TargetEligibility: Send + 'static {
    fn check(&self) -> Result<()>;
}

pub struct TargetWindow(usize);

impl TargetWindow {
    pub fn new(window: HWND) -> Self {
        Self(window as usize)
    }
}

impl TargetEligibility for TargetWindow {
    fn check(&self) -> Result<()> {
        let window = self.0 as HWND;
        unsafe {
            if IsWindow(window) == 0
                || IsWindowVisible(window) == 0
                || IsIconic(window) != 0
                || GetForegroundWindow() != window
            {
                return Err(OverlayError::TargetNotForeground);
            }
            if is_exclusive_fullscreen(window)? {
                return Err(OverlayError::ExclusiveFullscreen);
            }
        }
        Ok(())
    }
}

enum Command {
    Show(Pulse),
    Hide,
    Target(Rect),
    Topmost,
    Shutdown,
}
struct Request {
    command: Command,
    reply: SyncSender<Result<()>>,
}

/// The window and all GDI objects stay on the worker. Calls are synchronous
/// acknowledgements; animation is driven by a Win32 timer, idle time blocks in
/// MsgWaitForMultipleObjectsEx. Drop shuts down and joins after platform failure.
pub struct Win32Overlay {
    sender: SyncSender<Request>,
    wake: Arc<Wake>,
    window: usize,
    worker: Option<JoinHandle<Result<()>>>,
}

impl Win32Overlay {
    pub fn new(
        target: Rect,
        eligibility: impl TargetEligibility,
        options: Options,
    ) -> Result<Self> {
        Surface::for_target(target)?;
        let (sender, receiver) = mpsc::sync_channel(1);
        let (ready_tx, ready_rx) = mpsc::sync_channel(1);
        let wake = Arc::new(Wake::new()?);
        let worker_wake = Arc::clone(&wake);
        let worker = thread::Builder::new()
            .name("lucas-overlay".into())
            .spawn(move || {
                let mut native = match Native::new(target, Box::new(eligibility), options) {
                    Ok(native) => native,
                    Err(error) => {
                        let _ = ready_tx.send(Err(error.clone()));
                        return Err(error);
                    }
                };
                if ready_tx.send(Ok(native.window as usize)).is_err() {
                    return Ok(());
                }
                native.run(receiver, &worker_wake)
            })
            .map_err(|e| OverlayError::Platform {
                operation: "spawn overlay thread",
                code: e.raw_os_error().unwrap_or(0) as u32,
            })?;
        match ready_rx.recv() {
            Ok(Ok(window)) => Ok(Self {
                sender,
                wake,
                window,
                worker: Some(worker),
            }),
            outcome => {
                let joined = worker.join();
                match outcome {
                    Ok(Err(error)) => Err(error),
                    _ if joined.is_err() => Err(OverlayError::ThreadPanicked),
                    _ => Err(OverlayError::Closed),
                }
            }
        }
    }

    /// Borrowed native handle for diagnostics only. Do not destroy or mutate it;
    /// it ceases to be valid at shutdown or if the worker fails.
    pub fn window_handle(&self) -> HWND {
        self.window as HWND
    }

    fn request(&self, command: Command) -> Result<()> {
        if self
            .worker
            .as_ref()
            .is_none_or(|worker| worker.is_finished())
        {
            return Err(OverlayError::Closed);
        }
        let (reply, receive) = mpsc::sync_channel(1);
        self.sender
            .send(Request { command, reply })
            .map_err(|_| OverlayError::Closed)?;
        // A kernel event cannot lose a wake to the window-message quota. The
        // event stays owned by both sides until the worker has been joined.
        self.wake.signal()?;
        receive.recv().map_err(|_| OverlayError::Closed)?
    }
}

impl Overlay for Win32Overlay {
    fn show_pulse(&mut self, pulse: Pulse) -> Result<()> {
        self.request(Command::Show(pulse))
    }
    fn hide(&mut self) -> Result<()> {
        self.request(Command::Hide)
    }
    fn set_target_rect(&mut self, rect: Rect) -> Result<()> {
        if self.worker.is_none() {
            return Err(OverlayError::Closed);
        }
        Surface::for_target(rect)?;
        self.request(Command::Target(rect))
    }
    fn reassert_topmost(&mut self) -> Result<()> {
        self.request(Command::Topmost)
    }
    fn shutdown(&mut self) -> Result<()> {
        if self.worker.is_none() {
            return Ok(());
        }
        let sent = self.request(Command::Shutdown);
        self.window = 0;
        let result = self
            .worker
            .take()
            .unwrap()
            .join()
            .map_err(|_| OverlayError::ThreadPanicked)?;
        result?;
        sent
    }
}

impl Drop for Win32Overlay {
    fn drop(&mut self) {
        let _ = self.shutdown();
    }
}

// A kernel event is safe to signal/wait across threads. Store its address-sized
// value (not a window pointer) and close it only after the last Arc is dropped.
struct Wake(usize);
impl Wake {
    fn new() -> Result<Self> {
        // SAFETY: unnamed auto-reset event, no security descriptor or input data.
        let handle = unsafe { CreateEventW(null(), 0, 0, null()) };
        if handle.is_null() {
            return Err(last_error("CreateEventW"));
        }
        Ok(Self(handle as usize))
    }
    fn signal(&self) -> Result<()> {
        // SAFETY: this Arc-owned event cannot be closed during the call.
        if unsafe { SetEvent(self.0 as HANDLE) } == 0 {
            return Err(last_error("SetEvent"));
        }
        Ok(())
    }
}
impl Drop for Wake {
    fn drop(&mut self) {
        // SAFETY: last owner; no pending waits or signals can use the handle.
        unsafe {
            CloseHandle(self.0 as HANDLE);
        }
    }
}

struct Native {
    window: HWND,
    instance: HINSTANCE,
    class: Vec<u16>,
    surface: Surface,
    dib: Dib,
    pulse: Option<(Pulse, Instant)>,
    eligibility: Box<dyn TargetEligibility>,
}

impl Native {
    fn new(
        target: Rect,
        eligibility: Box<dyn TargetEligibility>,
        options: Options,
    ) -> Result<Self> {
        let surface = Surface::for_target(target)?;
        // SAFETY: all objects below are created and destroyed on this thread.
        // Setting thread DPI before creating any window avoids changing a host
        // application's process-wide awareness (which may already be fixed).
        unsafe {
            if SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_null() {
                return Err(last_error("SetThreadDpiAwarenessContext"));
            }
            let instance = GetModuleHandleW(null());
            if instance.is_null() {
                return Err(last_error("GetModuleHandleW"));
            }
            let class = wide(&format!("LUCAS.Overlay.{}", GetCurrentThreadId()));
            let definition = WNDCLASSW {
                lpfnWndProc: Some(window_proc),
                hInstance: instance,
                lpszClassName: class.as_ptr(),
                ..Default::default()
            };
            let dib = Dib::new(surface)?;
            if RegisterClassW(&definition) == 0 {
                return Err(last_error("RegisterClassW"));
            }
            let window = CreateWindowExW(
                EX_STYLE,
                class.as_ptr(),
                wide("LUCAS minimap pulse").as_ptr(),
                WS_POPUP,
                surface.left(),
                surface.top(),
                surface.width() as i32,
                surface.height() as i32,
                null_mut(),
                null_mut(),
                instance,
                null(),
            );
            if window.is_null() {
                let error = last_error("CreateWindowExW");
                UnregisterClassW(class.as_ptr(), instance);
                return Err(error);
            }
            let native = Self {
                window,
                instance,
                class,
                surface,
                dib,
                pulse: None,
                eligibility,
            };
            if options.exclude_from_capture
                && SetWindowDisplayAffinity(window, WDA_EXCLUDEFROMCAPTURE) == 0
            {
                return Err(last_error("SetWindowDisplayAffinity"));
            }
            Ok(native)
        }
    }

    fn run(&mut self, receiver: Receiver<Request>, wake: &Wake) -> Result<()> {
        // SAFETY: the wake handle lives through the entire loop, message storage
        // is valid, and dispatch targets only this thread's windows.
        unsafe {
            let mut message = MSG::default();
            loop {
                if self.pulse.is_some() && self.eligibility.check().is_err() {
                    self.hide();
                }
                while let Ok(request) = receiver.try_recv() {
                    let shutdown = matches!(request.command, Command::Shutdown);
                    let result = self.apply(request.command);
                    // On presentation failure never leave a stale reminder visible.
                    if result.is_err() {
                        self.hide();
                    }
                    let _ = request.reply.send(result);
                    if shutdown {
                        return Ok(());
                    }
                }
                // Bound each batch so a busy message queue cannot starve hide.
                for _ in 0..64 {
                    if PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) == 0 {
                        break;
                    }
                    if message.message == WM_QUIT {
                        return Ok(());
                    }
                    if message.hwnd == self.window
                        && message.message == WM_TIMER
                        && message.wParam == TIMER
                    {
                        if let Err(error) = self.draw() {
                            self.hide();
                            if !matches!(
                                error,
                                OverlayError::TargetNotForeground
                                    | OverlayError::ExclusiveFullscreen
                            ) {
                                return Err(error);
                            }
                        }
                    } else {
                        TranslateMessage(&message);
                        DispatchMessageW(&message);
                    }
                }
                let handle = wake.0 as HANDLE;
                if MsgWaitForMultipleObjectsEx(
                    1,
                    &handle,
                    if self.pulse.is_some() { 16 } else { INFINITE },
                    QS_ALLINPUT,
                    MWMO_INPUTAVAILABLE,
                ) == WAIT_FAILED
                {
                    return Err(last_error("MsgWaitForMultipleObjectsEx"));
                }
            }
        }
    }

    fn apply(&mut self, command: Command) -> Result<()> {
        match command {
            Command::Show(pulse) => {
                self.pulse = Some((pulse, Instant::now()));
                self.draw()?;
                self.eligibility.check()?;
                if self.pulse.is_none() {
                    return Ok(());
                }
                // SAFETY: a live window owned by this thread, timer has no callback.
                unsafe {
                    if SetTimer(self.window, TIMER, 16, None) == 0 {
                        return Err(last_error("SetTimer"));
                    }
                    ShowWindow(self.window, SW_SHOWNOACTIVATE);
                }
                self.topmost()
            }
            Command::Hide | Command::Shutdown => {
                self.hide();
                Ok(())
            }
            Command::Target(rect) => {
                let surface = Surface::for_target(rect)?;
                if surface == self.surface {
                    return Ok(());
                }
                let dib = Dib::new(surface)?;
                // SAFETY: no activation or z-order changes during repositioning.
                if unsafe {
                    SetWindowPos(
                        self.window,
                        null_mut(),
                        surface.left(),
                        surface.top(),
                        surface.width() as i32,
                        surface.height() as i32,
                        SWP_NOACTIVATE | SWP_NOZORDER,
                    )
                } == 0
                {
                    return Err(last_error("SetWindowPos(target)"));
                }
                self.surface = surface;
                self.dib = dib;
                self.draw()
            }
            Command::Topmost => {
                if self.pulse.is_some() {
                    self.eligibility.check()?;
                }
                self.topmost()
            }
        }
    }

    fn hide(&mut self) {
        self.pulse = None;
        // SAFETY: owned live window; ShowWindow returns previous visibility,
        // not success. KillTimer may legitimately find no timer on an idle window.
        unsafe {
            KillTimer(self.window, TIMER);
            ShowWindow(self.window, SW_HIDE);
        }
    }

    fn topmost(&self) -> Result<()> {
        // SAFETY: owned window, never activate or show an idle overlay.
        if unsafe {
            SetWindowPos(
                self.window,
                HWND_TOPMOST,
                0,
                0,
                0,
                0,
                SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
            )
        } == 0
        {
            return Err(last_error("SetWindowPos(topmost)"));
        }
        Ok(())
    }

    fn draw(&mut self) -> Result<()> {
        let Some((pulse, started)) = self.pulse else {
            return Ok(());
        };
        self.eligibility.check()?;
        let elapsed = started.elapsed();
        if elapsed >= Pulse::DURATION {
            self.hide();
            return Ok(());
        }
        let image = render::frame(self.surface, pulse, elapsed)?;
        self.eligibility.check()?;
        // SAFETY: DIB owns an exactly width*height*4 top-down 32-bit buffer. No
        // GDI operations use it while Rust writes. GdiFlush fences the prior use.
        unsafe {
            GdiFlush();
            let pixels = std::slice::from_raw_parts_mut(self.dib.bits, image.data().len());
            render::copy_bgra(&image, pixels);
            let position = POINT {
                x: self.surface.left(),
                y: self.surface.top(),
            };
            let size = SIZE {
                cx: self.surface.width() as i32,
                cy: self.surface.height() as i32,
            };
            let origin = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION {
                BlendOp: AC_SRC_OVER as u8,
                BlendFlags: 0,
                SourceConstantAlpha: 255,
                AlphaFormat: AC_SRC_ALPHA as u8,
            };
            if UpdateLayeredWindow(
                self.window,
                null_mut(),
                &position,
                &size,
                self.dib.dc,
                &origin,
                0,
                &blend,
                ULW_ALPHA,
            ) == 0
            {
                return Err(last_error("UpdateLayeredWindow"));
            }
        }
        Ok(())
    }
}

impl Drop for Native {
    fn drop(&mut self) {
        // SAFETY: same creating thread, window before class. Dib drops separately.
        unsafe {
            DestroyWindow(self.window);
            UnregisterClassW(self.class.as_ptr(), self.instance);
        }
    }
}

struct Dib {
    dc: HDC,
    bitmap: HBITMAP,
    previous: HGDIOBJ,
    bits: *mut u8,
}
impl Dib {
    fn new(surface: Surface) -> Result<Self> {
        // SAFETY: header describes bounded top-down BGRA storage. Every failure
        // path releases what it acquired; successful ownership transfers to Drop.
        unsafe {
            let dc = CreateCompatibleDC(null_mut());
            if dc.is_null() {
                return Err(last_error("CreateCompatibleDC"));
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
                let error = last_error("CreateDIBSection");
                DeleteDC(dc);
                return Err(error);
            }
            let previous = SelectObject(dc, bitmap);
            if previous.is_null() || previous as isize == GDI_ERROR as isize {
                let error = last_error("SelectObject");
                DeleteObject(bitmap);
                DeleteDC(dc);
                return Err(error);
            }
            Ok(Self {
                dc,
                bitmap,
                previous,
                bits: bits.cast(),
            })
        }
    }
}
impl Drop for Dib {
    fn drop(&mut self) {
        // SAFETY: deselect bitmap before deleting it and release the memory DC.
        unsafe {
            SelectObject(self.dc, self.previous);
            DeleteObject(self.bitmap);
            DeleteDC(self.dc);
        }
    }
}

unsafe extern "system" fn window_proc(window: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    match message {
        WM_MOUSEACTIVATE => MA_NOACTIVATE as LRESULT,
        WM_NCHITTEST => HTTRANSPARENT as LRESULT,
        // Geometry is in physical pixels and supplied by the caller's next poll,
        // not scaled from the WM_DPICHANGED suggested logical rectangle.
        WM_DPICHANGED | WM_CLOSE => 0,
        // SAFETY: forward untouched Win32 callback arguments to the default proc.
        _ => unsafe { DefWindowProcW(window, message, w, l) },
    }
}
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn last_error(operation: &'static str) -> OverlayError {
    // SAFETY: GetLastError is thread-local and has no pointer arguments.
    OverlayError::Platform {
        operation,
        code: unsafe { GetLastError() },
    }
}

/// Conservative exclusive-fullscreen hint, not a proof. Windows fullscreen
/// optimizations can make exclusive and borderless indistinguishable. Only a
/// foreground popup covering its monitor AND the shell's D3D-exclusive state
/// returns true; failure is reported rather than treated as borderless.
///
/// # Safety
/// `window` must be a valid borrowed HWND throughout this call. The caller must
/// use per-monitor-v2 DPI awareness when polling geometry.
pub unsafe fn is_exclusive_fullscreen(window: HWND) -> Result<bool> {
    // SAFETY: borrowed handle is supplied by the caller; all output buffers are
    // correctly sized. This reads only public window/monitor metadata.
    unsafe {
        if IsWindow(window) == 0 {
            return Err(OverlayError::Platform {
                operation: "IsWindow",
                code: ERROR_INVALID_WINDOW_HANDLE,
            });
        }
        if GetForegroundWindow() != window || IsIconic(window) != 0 {
            return Ok(false);
        }
        let mut state = 0;
        let hr = SHQueryUserNotificationState(&mut state);
        if hr < 0 {
            return Err(OverlayError::Platform {
                operation: "SHQueryUserNotificationState",
                code: hr as u32,
            });
        }
        if state != QUNS_RUNNING_D3D_FULL_SCREEN {
            return Ok(false);
        }
        SetLastError(0);
        let style = GetWindowLongPtrW(window, GWL_STYLE) as u32;
        if style == 0 && GetLastError() != 0 {
            return Err(last_error("GetWindowLongPtrW"));
        }
        if style & WS_POPUP == 0 || style & (WS_CAPTION | WS_THICKFRAME) != 0 {
            return Ok(false);
        }
        let monitor = MonitorFromWindow(window, MONITOR_DEFAULTTONEAREST);
        let mut info = MONITORINFO {
            cbSize: size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        if GetMonitorInfoW(monitor, &mut info) == 0 {
            return Err(last_error("GetMonitorInfoW"));
        }
        let mut rect = RECT::default();
        if GetWindowRect(window, &mut rect) == 0 {
            return Err(last_error("GetWindowRect"));
        }
        Ok(rect.left == info.rcMonitor.left
            && rect.top == info.rcMonitor.top
            && rect.right == info.rcMonitor.right
            && rect.bottom == info.rcMonitor.bottom)
    }
}
