//! Interactive desktop tests: ONLY the synthetic child window receives input or
//! is captured. Never point this harness at a real game. Explicitly ignored in
//! normal test runs; CI invokes `windows_overlay_smoke --ignored` separately.
#![cfg(windows)]
#![deny(unsafe_op_in_unsafe_fn)]

use lucas_geometry::{Alignment, MinimapSettings, minimap_rect};
use lucas_overlay::{
    Overlay, OverlayError, Pulse, Rect, Salience,
    render::Surface,
    win32::{Options, TargetEligibility, TargetWindow, Win32Overlay, is_exclusive_fullscreen},
};
use std::{
    path::PathBuf,
    process::{Child, Command},
    ptr::{null, null_mut},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::{Dwm::DwmFlush, Gdi::*},
    System::LibraryLoader::GetModuleHandleW,
    UI::{HiDpi::*, Input::KeyboardAndMouse::*, WindowsAndMessaging::*},
};

const RAISE: u32 = WM_APP + 20;
const MOVE: u32 = WM_APP + 21;
const FOCUS: u32 = WM_APP + 22;
const CLICK_PROPERTY: &str = "LUCAS.smoke.clicks";
static CLICKS: AtomicUsize = AtomicUsize::new(0);
static NEXT_GAME: AtomicUsize = AtomicUsize::new(0);

struct TestTarget {
    window: TargetWindow,
    exclusive: Arc<AtomicBool>,
}

impl TargetEligibility for TestTarget {
    fn check(&self) -> lucas_overlay::Result<()> {
        self.window.check()?;
        if self.exclusive.load(Ordering::SeqCst) {
            return Err(OverlayError::ExclusiveFullscreen);
        }
        Ok(())
    }
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

unsafe extern "system" fn game_proc(hwnd: HWND, message: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    // SAFETY: Win32 supplied this child process's valid window callback arguments.
    unsafe {
        match message {
            WM_LBUTTONDOWN => {
                let count = CLICKS.fetch_add(1, Ordering::SeqCst) + 1;
                SetPropW(hwnd, wide(CLICK_PROPERTY).as_ptr(), count as HANDLE);
                0
            }
            RAISE => {
                SetWindowPos(
                    hwnd,
                    HWND_TOPMOST,
                    0,
                    0,
                    0,
                    0,
                    SWP_NOACTIVATE | SWP_NOMOVE | SWP_NOSIZE,
                );
                0
            }
            FOCUS => {
                SetForegroundWindow(w as HWND);
                0
            }
            MOVE => {
                SetWindowPos(
                    hwnd,
                    null_mut(),
                    70,
                    40,
                    760,
                    560,
                    SWP_NOACTIVATE | SWP_NOZORDER,
                );
                0
            }
            WM_DESTROY => {
                RemovePropW(hwnd, wide(CLICK_PROPERTY).as_ptr());
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, message, w, l),
        }
    }
}

#[test]
#[ignore = "helper process, launched only by windows_overlay_smoke"]
fn fake_game_process() {
    let Ok(title) = std::env::var("LUCAS_SMOKE_CHILD_TITLE") else {
        return;
    };
    // SAFETY: all HWNDs, class strings and GDI objects are owned in this process.
    unsafe {
        assert!(
            !SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_null()
        );
        let instance = GetModuleHandleW(null());
        let class = wide("LUCAS.SmokeGame");
        let brush = CreateSolidBrush(0x00302820); // RGB(32, 40, 48), synthetic art only.
        let definition = WNDCLASSW {
            lpfnWndProc: Some(game_proc),
            hInstance: instance,
            lpszClassName: class.as_ptr(),
            hbrBackground: brush,
            ..Default::default()
        };
        assert_ne!(RegisterClassW(&definition), 0);
        let hwnd = CreateWindowExW(
            0,
            class.as_ptr(),
            wide(&title).as_ptr(),
            WS_POPUP | WS_VISIBLE,
            0,
            0,
            GetSystemMetrics(SM_CXSCREEN),
            GetSystemMetrics(SM_CYSCREEN),
            null_mut(),
            null_mut(),
            instance,
            null(),
        );
        assert!(!hwnd.is_null());
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
        let mut message = MSG::default();
        loop {
            let status = GetMessageW(&mut message, null_mut(), 0, 0);
            assert_ne!(status, -1);
            if status == 0 {
                break;
            }
            TranslateMessage(&message);
            DispatchMessageW(&message);
        }
        UnregisterClassW(class.as_ptr(), instance);
        DeleteObject(brush);
    }
}

struct Game {
    child: Child,
    window: HWND,
    cursor: POINT,
    artifacts: PathBuf,
}
impl Game {
    fn spawn() -> Self {
        let title = format!(
            "LUCAS fake borderless game {}-{}",
            std::process::id(),
            NEXT_GAME.fetch_add(1, Ordering::SeqCst)
        );
        let artifacts = std::env::var_os("LUCAS_OVERLAY_ARTIFACT_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                std::env::temp_dir().join(format!("lucas-overlay-smoke-{}", std::process::id()))
            });
        std::fs::create_dir_all(&artifacts).unwrap();
        let child = Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "fake_game_process", "--nocapture"])
            .env("LUCAS_SMOKE_CHILD_TITLE", &title)
            .spawn()
            .unwrap();
        let mut game = Self {
            child,
            window: null_mut(),
            cursor: POINT::default(),
            artifacts,
        };
        // SAFETY: local output pointer; foreground permission is granted solely
        // to the child we just spawned. FindWindow searches our unique title.
        unsafe {
            GetCursorPos(&mut game.cursor);
            AllowSetForegroundWindow(game.child.id());
            wait_for(
                || {
                    game.window =
                        FindWindowW(wide("LUCAS.SmokeGame").as_ptr(), wide(&title).as_ptr());
                    !game.window.is_null()
                },
                "fake game creation",
            );
            SendMessageW(game.window, RAISE, 0, 0);
            SetForegroundWindow(game.window);
            // Establish the fake game's focus with a real, bounded test click.
            click(game.window, POINT { x: 200, y: 200 });
            wait_for(
                || GetForegroundWindow() == game.window,
                "fake game foreground",
            );
            assert_ne!(
                RedrawWindow(
                    game.window,
                    null(),
                    null_mut(),
                    RDW_INVALIDATE | RDW_ERASE | RDW_UPDATENOW
                ),
                0
            );
        }
        game
    }

    fn snapshot(&self, name: &str) -> Snapshot {
        let snapshot = screenshot(self.window);
        snapshot
            .image
            .save_png(self.artifacts.join(format!("{name}.png")))
            .unwrap();
        snapshot
    }

    fn map(&self) -> Rect {
        // SAFETY: borrowed child HWND, live until Drop; this thread is PMv2-aware.
        unsafe {
            let mut r = RECT::default();
            let mut origin = POINT::default();
            assert_ne!(GetClientRect(self.window, &mut r), 0);
            assert_ne!(ClientToScreen(self.window, &mut origin), 0);
            let client = Rect::new(
                origin.x as f64,
                origin.y as f64,
                (origin.x + r.right) as f64,
                (origin.y + r.bottom) as f64,
            )
            .unwrap();
            minimap_rect(client, MinimapSettings::default(), Alignment::default()).unwrap()
        }
    }
}
impl Drop for Game {
    fn drop(&mut self) {
        // SAFETY: only our child window is closed and the saved cursor restored.
        unsafe {
            if !self.window.is_null() {
                PostMessageW(self.window, WM_CLOSE, 0, 0);
            }
            SetCursorPos(self.cursor.x, self.cursor.y);
        }
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if self.child.try_wait().ok().flatten().is_some() {
                return;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn wait_for(mut predicate: impl FnMut() -> bool, description: &str) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !predicate() {
        assert!(Instant::now() < deadline, "timed out: {description}");
        thread::sleep(Duration::from_millis(15));
    }
}

unsafe fn click(expected: HWND, point: POINT) {
    // SAFETY: only synthesize input after hit-testing the synthetic window.
    unsafe {
        assert_eq!(
            WindowFromPoint(point),
            expected,
            "refuse to click anything except the fake game"
        );
        assert_ne!(SetCursorPos(point.x, point.y), 0);
        let inputs = [MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP].map(|flags| INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dwFlags: flags,
                    ..Default::default()
                },
            },
        });
        assert_eq!(
            SendInput(
                inputs.len() as u32,
                inputs.as_ptr(),
                size_of::<INPUT>() as i32
            ),
            2
        );
    }
}

fn above(first: HWND, second: HWND) -> bool {
    // SAFETY: enumeration of top-level windows reads public desktop z-order only.
    unsafe {
        let mut window = GetTopWindow(null_mut());
        while !window.is_null() {
            if window == first {
                return true;
            }
            if window == second {
                return false;
            }
            window = GetWindow(window, GW_HWNDNEXT);
        }
    }
    panic!("test windows missing from z-order");
}

fn assert_bounds(window: HWND, target: Rect) {
    let s = Surface::for_target(target).unwrap();
    let mut rect = RECT::default();
    // SAFETY: caller supplies the live overlay's borrowed handle.
    unsafe {
        assert_ne!(GetWindowRect(window, &mut rect), 0);
    }
    assert_eq!(
        (rect.left, rect.top, rect.right, rect.bottom),
        (
            s.left(),
            s.top(),
            s.left() + s.width() as i32,
            s.top() + s.height() as i32
        )
    );
}

struct Snapshot {
    image: tiny_skia::Pixmap,
    bounds: RECT,
}

impl Snapshot {
    fn pixel(&self, x: i32, y: i32) -> Option<tiny_skia::PremultipliedColorU8> {
        self.image.pixel(
            u32::try_from(x - self.bounds.left).ok()?,
            u32::try_from(y - self.bounds.top).ok()?,
        )
    }
}

fn ring_pixel(image: &Snapshot, target: Rect) -> POINT {
    let s = Surface::for_target(target).unwrap();
    for y in s.top().max(image.bounds.top)..(s.top() + s.height() as i32).min(image.bounds.bottom) {
        for x in
            s.left().max(image.bounds.left)..(s.left() + s.width() as i32).min(image.bounds.right)
        {
            if target.contains(x as f64, y as f64) {
                continue;
            }
            let p = image.pixel(x, y).unwrap();
            if p.red() as u16 > p.green() as u16 + 20 && p.green() as u16 > p.blue() as u16 + 20 {
                return POINT { x, y };
            }
        }
    }
    panic!("CAPTUREBLT screenshot contains no amber ring near target");
}

#[test]
#[ignore = "requires interactive Windows desktop; run explicitly in windows-latest CI"]
fn windows_overlay_smoke() {
    // SAFETY: the test owns only synthetic windows; no game processes are touched.
    unsafe {
        let previous_dpi = SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        assert!(!previous_dpi.is_null());
        let game = Game::spawn();
        let target = game.map();
        let baseline = game.snapshot("01-fake-game");
        let background = baseline.pixel(200, 200).unwrap();
        assert_eq!(
            (background.red(), background.green(), background.blue()),
            (32, 40, 48),
            "fake window must be painted before taking the reference"
        );
        let exclusive = Arc::new(AtomicBool::new(false));
        let mut overlay = Win32Overlay::new(
            target,
            TestTarget {
                window: TargetWindow::new(game.window),
                exclusive: Arc::clone(&exclusive),
            },
            Options::default(),
        )
        .unwrap();
        let hwnd = overlay.window_handle();
        assert_eq!(
            GetForegroundWindow(),
            game.window,
            "creation must not activate"
        );
        assert_eq!(IsWindowVisible(hwnd), 0);
        let required =
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOPMOST | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW;
        assert_eq!(
            GetWindowLongPtrW(hwnd, GWL_EXSTYLE) as u32 & required,
            required
        );
        assert_ne!(GetWindowLongPtrW(hwnd, GWL_STYLE) as u32 & WS_POPUP, 0);
        assert_eq!(
            SendMessageW(hwnd, WM_MOUSEACTIVATE, 0, 0),
            MA_NOACTIVATE as isize
        );
        assert_ne!(
            AreDpiAwarenessContextsEqual(
                GetWindowDpiAwarenessContext(hwnd),
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2
            ),
            0
        );
        let mut affinity = u32::MAX;
        assert_ne!(GetWindowDisplayAffinity(hwnd, &mut affinity), 0);
        assert_eq!(affinity, WDA_NONE);
        assert_bounds(hwnd, target);
        assert!(
            Surface::for_target(target).unwrap().width() < baseline.image.width(),
            "overlay must stay minimap-sized"
        );
        assert_eq!(
            overlay.set_target_rect(Rect::new(0.0, 0.0, 1e30, 1e30).unwrap()),
            Err(OverlayError::InvalidRect)
        );
        assert_bounds(hwnd, target);
        assert!(
            !is_exclusive_fullscreen(game.window).unwrap(),
            "a borderless fake is not exclusive"
        );
        let pulse = Pulse {
            salience: Salience::Strong,
            ..Pulse::default()
        };
        overlay.show_pulse(pulse).unwrap();
        assert_eq!(GetForegroundWindow(), game.window, "show stole foreground");
        thread::sleep(Duration::from_millis(180));
        let shown = game.snapshot("02-ripple");
        let point = ring_pixel(&shown, target);
        assert_eq!(
            WindowFromPoint(point),
            game.window,
            "opaque ring pixels must be click-through"
        );
        let before = GetPropW(game.window, wide(CLICK_PROPERTY).as_ptr()) as usize;
        click(game.window, point);
        wait_for(
            || GetPropW(game.window, wide(CLICK_PROPERTY).as_ptr()) as usize > before,
            "SendInput click received by fake game",
        );
        // Verify the transparent map is exactly unchanged, not only its centre.
        for y in target.top().ceil() as i32..target.bottom().floor() as i32 {
            for x in target.left().ceil() as i32..target.right().floor() as i32 {
                assert_eq!(
                    shown.pixel(x, y),
                    baseline.pixel(x, y),
                    "overlay painted minimap content"
                );
            }
        }
        overlay.hide().unwrap();
        assert_eq!(GetForegroundWindow(), game.window, "hide stole foreground");
        assert_eq!(IsWindowVisible(hwnd), 0);
        game.snapshot("03-hidden");

        overlay.show_pulse(pulse).unwrap();
        SendMessageW(game.window, RAISE, 0, 0); // fake game raises ITSELF
        assert!(!above(hwnd, game.window));
        overlay.reassert_topmost().unwrap();
        assert!(above(hwnd, game.window));
        assert_eq!(GetForegroundWindow(), game.window);
        thread::sleep(Duration::from_millis(180));
        ring_pixel(&game.snapshot("04-topmost-reasserted"), target);

        let other = Game::spawn();
        SendMessageW(other.window, FOCUS, game.window as WPARAM, 0);
        wait_for(
            || GetForegroundWindow() == game.window,
            "focus pulse target",
        );
        overlay.show_pulse(pulse).unwrap();
        assert_ne!(IsWindowVisible(hwnd), 0);
        let lost_at = Instant::now();
        SendMessageW(game.window, FOCUS, other.window as WPARAM, 0);
        wait_for(
            || GetForegroundWindow() == other.window,
            "focus other fixture",
        );
        wait_for(|| IsWindowVisible(hwnd) == 0, "foreground loss hides pulse");
        assert!(lost_at.elapsed() < Duration::from_millis(500));
        assert_eq!(
            overlay.show_pulse(pulse),
            Err(OverlayError::TargetNotForeground)
        );
        assert_eq!(IsWindowVisible(hwnd), 0);
        assert_eq!(GetForegroundWindow(), other.window);
        SendMessageW(other.window, FOCUS, game.window as WPARAM, 0);
        wait_for(
            || GetForegroundWindow() == game.window,
            "restore fake game focus",
        );
        drop(other);
        assert_eq!(IsWindowVisible(hwnd), 0);
        overlay.show_pulse(pulse).unwrap();
        assert_ne!(IsWindowVisible(hwnd), 0);
        let exclusive_at = Instant::now();
        exclusive.store(true, Ordering::SeqCst);
        wait_for(|| IsWindowVisible(hwnd) == 0, "exclusive mode hides pulse");
        assert!(exclusive_at.elapsed() < Duration::from_millis(500));
        let error = overlay.show_pulse(pulse).unwrap_err();
        assert_eq!(error, OverlayError::ExclusiveFullscreen);
        assert!(error.to_string().contains("Borderless"));
        assert_eq!(IsWindowVisible(hwnd), 0);
        exclusive.store(false, Ordering::SeqCst);
        overlay.show_pulse(pulse).unwrap();
        assert_ne!(IsWindowVisible(hwnd), 0);

        SendMessageW(game.window, MOVE, 0, 0);
        let moved = game.map();
        assert_ne!(moved, target);
        overlay.set_target_rect(moved).unwrap(); // application poll supplies new geometry
        assert_bounds(hwnd, moved);
        assert_eq!(GetForegroundWindow(), game.window);
        overlay.show_pulse(pulse).unwrap();
        thread::sleep(Duration::from_millis(180));
        let resized = game.snapshot("05-moved-resized");
        assert_eq!((resized.image.width(), resized.image.height()), (760, 560));
        assert_eq!((resized.bounds.left, resized.bounds.top), (70, 40));
        ring_pixel(&resized, moved);
        wait_for(|| IsWindowVisible(hwnd) == 0, "finite pulse hides itself");
        game.snapshot("06-completed");
        overlay.reassert_topmost().unwrap();
        assert_eq!(
            IsWindowVisible(hwnd),
            0,
            "topmost request must not show an idle overlay"
        );
        overlay.shutdown().unwrap();
        assert_eq!(IsWindow(hwnd), 0);
        overlay.shutdown().unwrap();
        assert_eq!(overlay.show_pulse(pulse), Err(OverlayError::Closed));
        assert_eq!(GetForegroundWindow(), game.window);

        // Exercise opt-in affinity separately; it is OFF for screenshot assertions.
        let mut excluded = Win32Overlay::new(
            moved,
            TargetWindow::new(game.window),
            Options {
                exclude_from_capture: true,
            },
        )
        .unwrap();
        assert_ne!(
            GetWindowDisplayAffinity(excluded.window_handle(), &mut affinity),
            0
        );
        assert_eq!(affinity, WDA_EXCLUDEFROMCAPTURE);
        excluded.shutdown().unwrap();
        let dropped = {
            let temporary =
                Win32Overlay::new(moved, TargetWindow::new(game.window), Options::default())
                    .unwrap();
            temporary.window_handle()
        };
        assert_eq!(
            IsWindow(dropped),
            0,
            "Drop must join and destroy the window"
        );
        drop(game);
        SetThreadDpiAwarenessContext(previous_dpi);
    }
}

struct Capture {
    screen: HDC,
    memory: HDC,
    bitmap: HBITMAP,
    old: HGDIOBJ,
}
impl Drop for Capture {
    fn drop(&mut self) {
        // SAFETY: reverse acquisition order, deselect before deleting the bitmap.
        unsafe {
            if !self.old.is_null() {
                SelectObject(self.memory, self.old);
            }
            if !self.bitmap.is_null() {
                DeleteObject(self.bitmap);
            }
            if !self.memory.is_null() {
                DeleteDC(self.memory);
            }
            if !self.screen.is_null() {
                ReleaseDC(null_mut(), self.screen);
            }
        }
    }
}
fn screenshot(window: HWND) -> Snapshot {
    unsafe {
        assert!(!window.is_null());
        assert_eq!(GetForegroundWindow(), window);
        assert_ne!(IsWindowVisible(window), 0);
        assert_eq!(IsIconic(window), 0);
        let mut bounds = RECT::default();
        assert_ne!(GetWindowRect(window, &mut bounds), 0);
        let width = bounds.right - bounds.left;
        let height = bounds.bottom - bounds.top;
        assert!(width > 0 && height > 0);
        let mut capture = Capture {
            screen: GetDC(null_mut()),
            memory: null_mut(),
            bitmap: null_mut(),
            old: null_mut(),
        };
        assert!(!capture.screen.is_null());
        capture.memory = CreateCompatibleDC(capture.screen);
        assert!(!capture.memory.is_null());
        let info = BITMAPINFO {
            bmiHeader: BITMAPINFOHEADER {
                biSize: size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: width,
                biHeight: -height,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..Default::default()
            },
            ..Default::default()
        };
        let mut bits = null_mut();
        capture.bitmap = CreateDIBSection(
            capture.screen,
            &info,
            DIB_RGB_COLORS,
            &mut bits,
            null_mut(),
            0,
        );
        assert!(!capture.bitmap.is_null());
        capture.old = SelectObject(capture.memory, capture.bitmap);
        assert!(!capture.old.is_null());
        assert_eq!(DwmFlush(), 0, "wait for the compositor before capture");
        assert_ne!(
            BitBlt(
                capture.memory,
                0,
                0,
                width,
                height,
                capture.screen,
                bounds.left,
                bounds.top,
                SRCCOPY | CAPTUREBLT
            ),
            0
        );
        GdiFlush();
        let source =
            std::slice::from_raw_parts(bits.cast::<u8>(), width as usize * height as usize * 4);
        let mut image = tiny_skia::Pixmap::new(width as u32, height as u32).unwrap();
        for (bgra, rgba) in source
            .chunks_exact(4)
            .zip(image.data_mut().chunks_exact_mut(4))
        {
            rgba.copy_from_slice(&[bgra[2], bgra[1], bgra[0], 255]);
        }
        Snapshot { image, bounds }
    }
}
