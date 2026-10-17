//! Thin Windows adapter: tray/message pump, bounded API worker and overlay calls.
use super::{
    alignment::AlignmentSession,
    alignment_store::{self, AlignmentStore, Key},
    alignment_window::{self, KeyAction, Preview},
    report::GeometryReport,
    scheduling::{GateMode, Inputs, Status, Timer},
    settings::read_hud,
};
use lucas_game::{
    GameState, LiveClient, LiveDataSource, SettingsLocator, Win32WindowProbe, WindowProbe,
};
use lucas_geometry::{Rect, minimap_rect};
use lucas_overlay::{
    Overlay, OverlayError, Pulse,
    win32::{Options, TargetWindow, Win32Overlay},
};
use std::{
    error::Error,
    io,
    path::PathBuf,
    ptr::{null, null_mut},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, Instant},
};
use tray_icon::{
    Icon, TrayIconBuilder,
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem},
};
use windows_sys::Win32::{
    Foundation::*,
    System::Threading::CreateMutexW,
    UI::{HiDpi::*, WindowsAndMessaging::*},
};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub fn run() -> Result<()> {
    let Some(_instance) = SingleInstance::acquire()? else {
        return Ok(());
    };
    // SAFETY: before any tray or overlay window on this thread. Physical window
    // geometry must not be DPI virtualized. The overlay sets its own thread DPI.
    if unsafe { SetThreadDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2) }.is_null()
    {
        return Err(io::Error::last_os_error().into());
    }
    let folder = alignment_store::data_folder()?;
    std::fs::create_dir_all(&folder)?;
    // Do not overwrite a malformed/unsupported file with defaults.
    let mut store = AlignmentStore::load(&folder)?;
    let locator =
        SettingsLocator::windows_defaults(std::env::var_os("LUCAS_LEAGUE_ROOT").map(PathBuf::from));
    let origin = Instant::now();
    let worker = ApiWorker::start(origin)?;
    let mut timer = Timer::default();
    let mut probe = Win32WindowProbe;
    let mut hud = None;
    let mut settings_due = Duration::ZERO;
    let mut overlay: Option<(usize, Win32Overlay)> = None;
    let mut alignment: Option<(AlignmentSession, Preview)> = None;
    let mut last_key = None;
    let mut last_rect = None;

    let status_item = MenuItem::new("Waiting for a game", false, None);
    let notice_item = MenuItem::new("Timer mode: amber ripple every 10 seconds", false, None);
    let pause = MenuItem::new("Pause pulses", true, None);
    let align = MenuItem::new("Align minimap...", false, None);
    let report = MenuItem::new("Save minimap numbers", false, None);
    let live_gate = CheckMenuItem::new("Use Live Client gate (recommended)", true, true, None);
    let open = MenuItem::new("Open data folder", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::with_items(&[
        &status_item,
        &notice_item,
        &PredefinedMenuItem::separator(),
        &pause,
        &align,
        &report,
        &live_gate,
        &PredefinedMenuItem::separator(),
        &open,
        &quit,
    ])?;
    let tray = TrayIconBuilder::new()
        .with_icon(icon()?)
        .with_tooltip("LUCAS — waiting for a game")
        .with_menu(Box::new(menu))
        .build()?;
    let mut last_status = String::new();
    let mut quit_requested = false;
    let mut last_tick = origin.elapsed();

    while !quit_requested {
        let mut keys = Vec::new();
        // SAFETY: dispatch only this thread's messages. No hooks or synthesized
        // input. The native tray menu needs a continuously serviced Win32 queue.
        unsafe {
            let mut message = MSG::default();
            for _ in 0..128 {
                if PeekMessageW(&mut message, null_mut(), 0, 0, PM_REMOVE) == 0 {
                    break;
                }
                if message.message == WM_QUIT {
                    quit_requested = true;
                    break;
                }
                if message.message == WM_HOTKEY
                    && alignment
                        .as_ref()
                        .is_some_and(|(_, p)| p.handle() == message.hwnd)
                {
                    if let Some(action) = alignment_window::action(message.wParam) {
                        keys.push(action);
                    }
                } else {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
        // Sample after dispatch: opening a tray menu can run a nested loop for
        // seconds. Never use pre-menu timestamps or stale foreground information.
        let now = origin.elapsed();
        // A tray popup runs a nested message loop. If it held this tick, do not
        // credit the time spent away from League toward the next reminder.
        if now.saturating_sub(last_tick) > Duration::from_millis(500) {
            timer.reset();
        }
        last_tick = now;
        let window = probe.sample().unwrap_or_default();
        let target = game_window();
        if now >= settings_due {
            hud = read_hud(&locator).ok();
            settings_due = now + Duration::from_secs(1);
        }
        let context = window
            .client_rect
            .and_then(|r| {
                Rect::new(r.left as f64, r.top as f64, r.right as f64, r.bottom as f64).ok()
            })
            .zip(hud);
        let key = context.map(|(client, settings)| Key::new(client, settings));
        if last_key != key
            || overlay
                .as_ref()
                .is_some_and(|(old, _)| *old != target as usize)
        {
            alignment = None;
            overlay = None;
            last_rect = None;
            timer.reset();
            last_key = key;
        }
        let game = worker.eligibility(origin);
        let game_allowed = timer.mode() == GateMode::WindowOnly || game.eligible();
        align.set_enabled(context.is_some() && window.exists && game_allowed);
        report.set_enabled(context.is_some() && window.exists && alignment.is_none());

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == *quit.id() {
                quit_requested = true;
            } else if event.id == *pause.id() {
                timer.set_paused(!timer.paused());
                alignment = None;
                pause.set_text(if timer.paused() {
                    "Resume pulses"
                } else {
                    "Pause pulses"
                });
                notice_item.set_text("Timer mode: amber ripple every 10 seconds");
            } else if event.id == *live_gate.id() {
                let mode = if live_gate.is_checked() {
                    GateMode::LiveClient
                } else {
                    GateMode::WindowOnly
                };
                timer.set_mode(mode);
                worker.set_enabled(mode == GateMode::LiveClient);
                alignment = None;
                notice_item.set_text(if mode == GateMode::WindowOnly {
                    "Window-only: pulses also run while dead/loading; pause manually"
                } else {
                    "Live Client gate: alive Summoner's Rift, after 1:30"
                });
            } else if event.id == *align.id() {
                if let Some(key) = key
                    && !target.is_null()
                    && game_allowed
                {
                    alignment = None;
                    match Preview::new(target) {
                        Ok(preview) => {
                            alignment = Some((AlignmentSession::new(key, store.get(key)), preview));
                            timer.reset();
                            notice_item.set_text("Return to League: arrows move; Shift+arrows resize; Enter saves; Esc cancels");
                        }
                        Err(error) => {
                            notice_item.set_text(format!("Alignment unavailable: {error}"))
                        }
                    }
                }
            } else if event.id == *report.id() {
                if let Some((client, settings)) = context
                    && alignment.is_none()
                {
                    let result = GeometryReport::new(
                        client,
                        settings,
                        store.get(Key::new(client, settings)),
                    )
                    .map_err(io::Error::other)
                    .and_then(|r| r.save(&folder));
                    match result {
                        Ok(_) => notice_item.set_text("Numbers saved in data folder / reports"),
                        Err(error) => {
                            notice_item.set_text(format!("Could not save numbers: {error}"))
                        }
                    }
                }
            } else if event.id == *open.id() {
                // Direct executable + argument, never shell interpolation.
                if let Err(error) = std::process::Command::new("explorer.exe")
                    .arg(&folder)
                    .spawn()
                {
                    notice_item.set_text(format!("Could not open data folder: {error}"));
                }
            }
        }
        // Re-evaluate after menu mode changes. API stale state is cleared there.
        let game = worker.eligibility(origin);
        let game_allowed = timer.mode() == GateMode::WindowOnly || game.eligible();
        let alignment_visible = window.overlay_eligible() && game_allowed;
        for action in keys {
            // Queued keys after alt-tab or a profile change must do nothing.
            if !alignment_visible {
                break;
            }
            if let Some((session, _)) = alignment.as_mut()
                && let Some((client, settings)) = context
            {
                match action {
                    KeyAction::Adjust(action) => session.adjust(client, settings, action),
                    KeyAction::Cancel => {
                        alignment = None;
                        notice_item.set_text("Alignment cancelled; saved profile unchanged");
                    }
                    KeyAction::Save => {
                        let mut updated = store.clone();
                        let result = updated
                            .set(session.key, session.draft)
                            .and_then(|()| updated.save(&folder));
                        match result {
                            Ok(()) => {
                                store = updated;
                                alignment = None;
                                notice_item.set_text(
                                    "Alignment saved for this resolution, scale and flip",
                                );
                            }
                            Err(error) => notice_item.set_text(format!(
                                "Alignment not saved; retry Enter or Esc: {error}"
                            )),
                        }
                    }
                }
            }
        }
        let mut rect = None;
        if let Some((client, settings)) = context {
            let correction = alignment
                .as_ref()
                .map(|(s, _)| s.draft)
                .unwrap_or_else(|| store.get(Key::new(client, settings)));
            rect = minimap_rect(client, settings, correction).ok();
        }
        if let Some((_, preview)) = alignment.as_mut() {
            if alignment_visible && let Some(rect) = rect {
                if let Err(error) = preview.show(rect) {
                    preview.hide();
                    alignment = None;
                    notice_item.set_text(format!(
                        "Alignment stopped: {error}. Try Align minimap again."
                    ));
                }
            } else {
                preview.hide();
            }
        }
        if overlay.is_none()
            && !target.is_null()
            && let Some(rect) = rect
        {
            match Win32Overlay::new(rect, TargetWindow::new(target), Options::default()) {
                Ok(native) => {
                    overlay = Some((target as usize, native));
                    last_rect = Some(rect);
                }
                Err(error) => {
                    notice_item.set_text(format!("Overlay unavailable: {error}"));
                }
            }
        }
        let mut geometry_ready = rect.is_some() && overlay.is_some();
        if let Some((_, native)) = overlay.as_mut()
            && let Some(rect) = rect
            && last_rect != Some(rect)
        {
            if let Err(error) = native.set_target_rect(rect) {
                geometry_ready = false;
                notice_item.set_text(format!("Cannot position overlay: {error}"));
            } else {
                last_rect = Some(rect);
            }
        }
        let decision = timer.tick(
            now,
            Inputs {
                game,
                window,
                geometry_ready,
                aligning: alignment.is_some(),
            },
        );
        if let Some((_, native)) = overlay.as_mut() {
            let result = if decision.hide {
                native.hide()
            } else if decision.pulse {
                native.show_pulse(Pulse::default())
            } else {
                native.reassert_topmost()
            };
            if let Err(error) = result {
                timer.reset();
                if !matches!(error, OverlayError::TargetNotForeground) {
                    notice_item.set_text(format!("Overlay stopped: {error}"));
                    overlay = None;
                }
            }
        }
        let text = if alignment.is_some() {
            "Paused — aligning minimap (Enter saves, Esc cancels)"
        } else if timer.paused() {
            "Paused"
        } else if window.exists && hud.is_none() {
            "Waiting — HUD settings unavailable; set LUCAS_LEAGUE_ROOT"
        } else if window.display_mode == lucas_game::DisplayMode::ExclusiveFullscreen {
            "Waiting — switch League to Borderless"
        } else {
            match decision.status {
                Status::Active => "Active — timer every 10 seconds",
                Status::Paused => "Paused",
                Status::Waiting => "Waiting for a game / alive player / League foreground",
            }
        };
        if text != last_status {
            status_item.set_text(text);
            tray.set_tooltip(Some(format!("LUCAS — {text}")))?;
            last_status = text.into();
        }
        // Sleeping is bounded; overlay has its own 16 ms foreground gate. The
        // preview additionally checks that same gate in its WM_TIMER handler.
        thread::sleep(Duration::from_millis(25));
    }
    drop(alignment);
    if let Some((_, mut overlay)) = overlay {
        overlay.shutdown()?;
    }
    drop(worker);
    drop(tray);
    Ok(())
}

pub(super) fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(Some(0)).collect()
}
fn game_window() -> HWND {
    // SAFETY: live terminated class string; no process handles or memory reads.
    unsafe { FindWindowW(wide("RiotWindowClass").as_ptr(), null()) }
}

struct SingleInstance(HANDLE);
impl SingleInstance {
    fn acquire() -> io::Result<Option<Self>> {
        // SAFETY: process-owned named mutex; do not acquire ownership, just hold
        // its lifetime. Local namespace is per interactive Windows session.
        unsafe {
            let handle = CreateMutexW(null(), 0, wide("Local\\LUCAS.Timer.v1").as_ptr());
            if handle.is_null() {
                return Err(io::Error::last_os_error());
            }
            if GetLastError() == ERROR_ALREADY_EXISTS {
                CloseHandle(handle);
                Ok(None)
            } else {
                Ok(Some(Self(handle)))
            }
        }
    }
}
impl Drop for SingleInstance {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}

fn icon() -> Result<Icon> {
    // Authored amber corner/ring mark, high contrast at notification-area size.
    let mut pixels = vec![0; 32 * 32 * 4];
    for y in 0..32 {
        for x in 0..32 {
            if (4..28).contains(&x)
                && (4..28).contains(&y)
                && (!(8..24).contains(&x) || !(8..24).contains(&y))
            {
                pixels[(y * 32 + x) * 4..(y * 32 + x + 1) * 4]
                    .copy_from_slice(&[255, 184, 56, 255]);
            }
        }
    }
    Ok(Icon::from_rgba(pixels, 32, 32)?)
}

struct ApiWorker {
    state: Arc<Mutex<GameState>>,
    generation: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl ApiWorker {
    fn start(origin: Instant) -> io::Result<Self> {
        let state = Arc::new(Mutex::new(GameState::new()));
        let generation = Arc::new(AtomicU64::new(1)); // low bit: enabled
        let stop = Arc::new(AtomicBool::new(false));
        let (s, g, stopped) = (
            Arc::clone(&state),
            Arc::clone(&generation),
            Arc::clone(&stop),
        );
        let handle = thread::Builder::new()
            .name("lucas-live-client".into())
            .spawn(move || {
                let mut client = LiveClient::new().ok();
                let mut next = Duration::ZERO;
                while !stopped.load(Ordering::Acquire) {
                    let token = g.load(Ordering::Acquire);
                    let now = origin.elapsed();
                    if token & 1 == 1 && now >= next {
                        if client.is_none() {
                            client = LiveClient::new().ok();
                        }
                        let result = client.as_mut().map(LiveDataSource::poll);
                        let completed = origin.elapsed();
                        next = completed + Duration::from_secs(1);
                        if let Ok(mut state) = s.lock()
                            && g.load(Ordering::Acquire) == token
                        {
                            let success = match result {
                                Some(Ok(snapshot)) => state.update(completed, &snapshot).is_ok(),
                                _ => false,
                            };
                            if !success {
                                let _ = state.unavailable(completed);
                            }
                        }
                    }
                    thread::sleep(Duration::from_millis(50));
                }
            })?;
        Ok(Self {
            state,
            generation,
            stop,
            handle: Some(handle),
        })
    }
    fn eligibility(&self, origin: Instant) -> lucas_game::Eligibility {
        self.state
            .lock()
            // Sample while holding the lock: the worker cannot install a poll
            // timestamp later than this freshness check's clock sample.
            .map(|s| s.eligibility(origin.elapsed()))
            .unwrap_or_default()
    }
    fn set_enabled(&self, enabled: bool) {
        let old = self.generation.load(Ordering::Acquire);
        self.generation
            .store(((old / 2 + 1) * 2) | u64::from(enabled), Ordering::Release);
        if let Ok(mut state) = self.state.lock() {
            state.reset();
        }
    }
}
impl Drop for ApiWorker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        if let Some(handle) = self.handle.take() {
            let _ = handle.join();
        }
    }
}
