//! Thin Windows adapter: tray/message pump, bounded API worker and overlay calls.
use super::{
    alignment::AlignmentSession,
    alignment_panel::KeyAction,
    alignment_store::{self, AlignmentStore, Key},
    alignment_window::Preview,
    report::GeometryReport,
    scheduling::{GateMode, Inputs, Status, Timer},
    settings::read_hud,
};
use lucas_game::{LiveClient, LiveDataSource, SettingsLocator, Win32WindowProbe, WindowProbe};
use lucas_geometry::{Rect, minimap_rect};
use lucas_overlay::{
    Overlay, OverlayError, Pulse,
    win32::{Options, TargetWindow, Win32Overlay},
};
use std::{
    collections::VecDeque,
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
    menu::{CheckMenuItem, Menu, MenuEvent, MenuItem, PredefinedMenuItem, Submenu},
};
use windows_sys::Win32::{
    Foundation::*,
    Graphics::Gdi::{GetMonitorInfoW, MONITOR_DEFAULTTOPRIMARY, MONITORINFO, MonitorFromPoint},
    System::{Diagnostics::Debug::MessageBeep, Threading::CreateMutexW},
    UI::{HiDpi::*, WindowsAndMessaging::*},
};

type Result<T> = std::result::Result<T, Box<dyn Error + Send + Sync>>;

pub fn run() -> Result<()> {
    run_mode(false)
}
pub fn run_timer() -> Result<()> {
    run_mode(true)
}
fn run_mode(timer_mode: bool) -> Result<()> {
    let Some(_instance) = SingleInstance::acquire()? else {
        return Ok(());
    };
    if run_session(timer_mode)? {
        lucas_store::Store::open()?.delete_all_data()?;
    }
    Ok(())
}

fn run_session(mut timer_mode: bool) -> Result<bool> {
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
    let mut gaze = crate::runtime::desktop::Desktop::new(origin)?;
    // Explicit --timer remains a no-camera diagnostic fallback. Normal launch
    // (or switching to gaze below) requires first-run setup.
    let mut setup_pending = !timer_mode && crate::preferences::needs_setup(gaze.runtime.settings());
    let mut timer = Timer::default();
    if !gaze.runtime.settings().live_client_gate {
        timer.set_mode(GateMode::WindowOnly);
        worker.set_enabled(false);
    }
    let mut probe = Win32WindowProbe;
    let mut hud = None;
    let mut settings_due = Duration::ZERO;
    let mut overlay: Option<(usize, Win32Overlay)> = None;
    let mut alignment: Option<(AlignmentSession, Preview)> = None;
    let mut last_key = None;
    let mut last_rect = None;

    let status_item = MenuItem::new("Waiting for a game", false, None);
    let notice_item = MenuItem::new(
        "Gaze mode: complete a quick check from the tray each game",
        false,
        None,
    );
    let timer_option = CheckMenuItem::new("Timer mode (no gaze)", true, timer_mode, None);
    let calibrate = MenuItem::new("Calibrate gaze...", true, None);
    let levels = Submenu::new("Training level (next game)", true);
    let level_items: Vec<_> = crate::preferences::selectable_levels()
        .map(|(i, level)| {
            (
                i,
                CheckMenuItem::new(
                    crate::preferences::level_label(level),
                    true,
                    i == gaze.runtime.settings().level_index,
                    None,
                ),
            )
        })
        .collect();
    for (_, item) in &level_items {
        levels.append(item)?;
    }
    let solo = CheckMenuItem::new("Next game solo (no pulses)", true, false, None);
    let preferences = MenuItem::new("Settings...", true, None);
    let setup = MenuItem::new("First-run setup...", true, None);
    let pause = MenuItem::new("Pause pulses", true, None);
    let align = MenuItem::new("Align minimap...", false, None);
    let report = MenuItem::new("Save minimap numbers", false, None);
    let live_gate = CheckMenuItem::new(
        "Use Live Client gate (recommended)",
        true,
        gaze.runtime.settings().live_client_gate,
        None,
    );
    let open = MenuItem::new("Open data folder", true, None);
    let quit = MenuItem::new("Quit", true, None);
    let menu = Menu::with_items(&[
        &status_item,
        &notice_item,
        &PredefinedMenuItem::separator(),
        &pause,
        &levels,
        &solo,
        &preferences,
        &setup,
        &timer_option,
        &calibrate,
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
    let mut delete_requested = false;
    let mut last_tick = origin.elapsed();
    // Compare the calibration display even before a game window exists, so a
    // changed monitor/DPI/alignment routes startup back to calibration.
    if let Some(screen) = primary_screen() {
        let client = Rect::new(
            screen.left as f64,
            screen.top as f64,
            screen.left as f64 + screen.width as f64,
            screen.top as f64 + screen.height as f64,
        )?;
        let settings = read_hud(&locator).unwrap_or_default();
        let rect = minimap_rect(client, settings, store.get(Key::new(client, settings)))?;
        gaze.geometry(screen, rect)?;
    }
    // Never steal game focus: quick checks and mid-game recalibration are tray actions.
    if setup_pending {
        gaze.launch_controls(true)?;
    } else if !timer_mode
        && !probe.sample().unwrap_or_default().exists
        && gaze.runtime.calibration() == crate::runtime::CalibrationState::Needed
    {
        gaze.launch_calibration()?;
    }

    while !quit_requested {
        if gaze.deletion_requested()? {
            delete_requested = true;
            break;
        }
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
                if !alignment
                    .as_mut()
                    .is_some_and(|(_, preview)| preview.process_message(&message))
                {
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
            || alignment
                .as_ref()
                .is_some_and(|(_, p)| p.target() != target)
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
        for (at, snapshot) in worker.drain() {
            gaze.poll(at, snapshot.as_ref())?;
        }
        // Drained polls may have completed after the earlier window sample.
        let now = origin.elapsed();
        let game = gaze.runtime.eligibility(now);
        let game_allowed = timer.mode() == GateMode::WindowOnly || game.eligible();
        let suppress = gaze.runtime.solo_active()
            || gaze.editing()
            || setup_pending
            || (gaze.runtime.solo_armed() && !gaze.runtime.in_session() && window.exists);
        if suppress {
            alignment = None;
        }
        align.set_enabled(context.is_some() && window.exists && game_allowed && !suppress);
        levels.set_enabled(!gaze.editing() && !gaze.calibrating() && !setup_pending);
        live_gate.set_enabled(!gaze.editing() && !gaze.calibrating() && !setup_pending);
        preferences.set_enabled(!gaze.editing() && !gaze.calibrating());
        setup.set_enabled(!gaze.editing() && !gaze.calibrating());
        report.set_enabled(context.is_some() && window.exists && alignment.is_none());

        while let Ok(event) = MenuEvent::receiver().try_recv() {
            if event.id == *quit.id() {
                quit_requested = true;
            } else if let Some((index, _)) =
                level_items.iter().find(|(_, item)| event.id == *item.id())
            {
                if gaze.editing() || gaze.calibrating() || setup_pending {
                    continue;
                }
                let mut updated = gaze.runtime.settings().clone();
                updated.level_index = *index;
                match lucas_store::Store::open().and_then(|s| s.save_settings(&updated)) {
                    Ok(()) => {
                        gaze.runtime.update_preferences(&updated)?;
                        notice_item
                            .set_text("Training level saved — applies next game, not this one");
                    }
                    Err(error) => notice_item.set_text(format!("Level not saved: {error}")),
                }
            } else if event.id == *solo.id() {
                gaze.runtime.arm_solo(solo.is_checked());
                notice_item.set_text(if gaze.runtime.solo_armed() {
                    "Next game is solo: no visual or sound pulses; summary after play"
                } else {
                    "Solo game disarmed"
                });
                last_status.clear(); // refresh the persistent tooltip announcement
            } else if event.id == *preferences.id() || event.id == *setup.id() {
                alignment = None;
                if let Err(error) = gaze.launch_controls(
                    crate::preferences::needs_setup(gaze.runtime.settings())
                        || event.id == *setup.id(),
                ) {
                    notice_item.set_text(format!("Cannot open settings: {error}"));
                }
            } else if event.id == *timer_option.id() {
                timer_mode = timer_option.is_checked();
                timer.reset();
                if !timer_mode && crate::preferences::needs_setup(gaze.runtime.settings()) {
                    setup_pending = true;
                    gaze.launch_controls(true)?;
                }
                notice_item.set_text(if timer_mode {
                    "Timer mode: no gaze; fixed 10-second reminders"
                } else {
                    "Gaze mode: quick check required each game"
                });
            } else if event.id == *calibrate.id() {
                if let Err(error) = gaze.launch_calibration() {
                    notice_item.set_text(format!("Calibration unavailable: {error}"));
                }
            } else if event.id == *pause.id() {
                timer.set_paused(!timer.paused());
                alignment = None;
                pause.set_text(if timer.paused() {
                    "Resume pulses"
                } else {
                    "Pause pulses"
                });
                notice_item.set_text("Pulse pause changed");
            } else if event.id == *live_gate.id() {
                if gaze.editing() || gaze.calibrating() || setup_pending {
                    continue;
                }
                let mode = if live_gate.is_checked() {
                    GateMode::LiveClient
                } else {
                    GateMode::WindowOnly
                };
                timer.set_mode(mode);
                worker.set_enabled(mode == GateMode::LiveClient);
                gaze.runtime.set_live_gate(
                    mode == GateMode::LiveClient,
                    now,
                    crate::runtime::desktop::unix_ms(),
                )?;
                lucas_store::Store::open()?.save_settings(gaze.runtime.settings())?;
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
                    && !suppress
                    && !gaze.editing()
                {
                    alignment = None;
                    let Some((client, settings)) = context else {
                        continue;
                    };
                    let Ok(rect) = minimap_rect(client, settings, store.get(key)) else {
                        continue;
                    };
                    match Preview::new(target, rect) {
                        Ok(preview) => {
                            alignment = Some((AlignmentSession::new(key, store.get(key)), preview));
                            timer.reset();
                            notice_item.set_text("Use the LUCAS alignment panel: arrows move; Shift+arrows resize; Save or Cancel");
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
        let game = gaze.runtime.eligibility(now);
        let game_allowed = timer.mode() == GateMode::WindowOnly || game.eligible();
        let alignment_visible = alignment.as_ref().is_some_and(|(_, p)| p.eligible())
            && game_allowed
            && !suppress
            && !gaze.editing();
        // Drain only the surviving panel: profile/window changes or reopening
        // alignment must discard the old panel's queued edits and Save action.
        let actions = alignment
            .as_mut()
            .map(|(_, p)| p.take_actions())
            .unwrap_or_default();
        for action in actions {
            // Cancel/close remains available even when the preview is ineligible.
            // Other queued input after alt-tab must not edit or save a draft.
            if !alignment_visible && action != KeyAction::Cancel {
                continue;
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
                                "Alignment not saved; retry Save or Cancel in the panel: {error}"
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
        if let (Some((client, _)), Some(rect)) = (context, rect) {
            // A changed window size, DPI, origin or aligned map rejects the fit.
            let dpi = unsafe { GetDpiForWindow(target) };
            if dpi > 0 {
                gaze.geometry(
                    lucas_store::ScreenGeometry {
                        left: client.left() as i32,
                        top: client.top() as i32,
                        width: client.width() as u32,
                        height: client.height() as u32,
                        scale_factor: dpi as f64 / 96.0,
                    },
                    rect,
                )?;
            }
        }
        let gaze_tick = gaze.tick(
            now,
            crate::runtime::Inputs {
                window_exists: window.exists,
                foreground: window.overlay_eligible(),
                geometry_ready,
                gaze_mode: (!timer_mode || gaze.runtime.solo_active()) && !setup_pending,
                paused: timer.paused() || alignment.is_some() || setup_pending,
            },
        )?;
        if gaze.deletion_requested()? {
            delete_requested = true;
            break;
        }
        calibrate.set_text(match gaze.runtime.calibration() {
            crate::runtime::CalibrationState::Needed => "Calibration needed / recalibrate...",
            crate::runtime::CalibrationState::QuickCheck => "Quick check required (4 seconds)...",
            crate::runtime::CalibrationState::Ready => "Recalibrate gaze...",
        });
        calibrate.set_enabled(!gaze.calibrating() && !gaze.editing() && !setup_pending);
        setup_pending = !timer_mode && crate::preferences::needs_setup(gaze.runtime.settings());
        if let Some(notice) = gaze.controls_notice.take() {
            notice_item.set_text(notice);
        }
        for (index, item) in &level_items {
            item.set_checked(*index == gaze.runtime.settings().level_index);
        }
        solo.set_checked(gaze.runtime.solo_armed());
        let decision = timer.tick(
            now,
            Inputs {
                game,
                window,
                geometry_ready,
                aligning: alignment.is_some()
                    || gaze.editing()
                    || setup_pending
                    || gaze.runtime.solo_active()
                    || gaze.calibrating(),
            },
        );
        if let Some((_, native)) = overlay.as_mut() {
            let result = if gaze.runtime.solo_active()
                || gaze.editing()
                || setup_pending
                || gaze.calibrating()
                || if timer_mode {
                    decision.hide
                } else {
                    gaze_tick.hide
                } {
                native.hide()
            } else if if timer_mode {
                decision.pulse
            } else {
                gaze_tick.pulse.is_some()
            } {
                let mut pulse = Pulse::default();
                if timer_mode {
                    gaze.runtime.timer_pulse(now)?;
                }
                {
                    let settings = &gaze.runtime.settings().pulse;
                    if let Some(request) = gaze_tick.pulse {
                        pulse.salience = request.salience;
                    }
                    pulse.colour = lucas_overlay::Colour(
                        settings.colour_rgb[0],
                        settings.colour_rgb[1],
                        settings.colour_rgb[2],
                    );
                    pulse.style = match settings.style {
                        lucas_store::PulseStyle::Ripple => lucas_overlay::PulseStyle::Ripple,
                        lucas_store::PulseStyle::BreathingBorder => {
                            lucas_overlay::PulseStyle::BreathingBorder
                        }
                        lucas_store::PulseStyle::CornerBrackets => {
                            lucas_overlay::PulseStyle::CornerBrackets
                        }
                        lucas_store::PulseStyle::CometSweep => {
                            lucas_overlay::PulseStyle::CometSweep
                        }
                    };
                }
                let shown = native.show_pulse(pulse);
                if shown.is_ok() && gaze.runtime.settings().pulse.sound {
                    unsafe {
                        MessageBeep(MB_OK);
                    }
                }
                shown
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
        let text = if gaze.runtime.solo_active() {
            "Solo game — no pulses or sound; summary after play"
        } else if gaze.editing() || setup_pending {
            "Setup / settings — coaching paused"
        } else if alignment.is_some() {
            "Paused — use LUCAS alignment panel (Save / Cancel)"
        } else if timer.paused() {
            "Paused"
        } else if window.exists && hud.is_none() {
            "Waiting — HUD settings unavailable; set LUCAS_LEAGUE_ROOT"
        } else if window.display_mode == lucas_game::DisplayMode::ExclusiveFullscreen {
            "Waiting — switch League to Borderless"
        } else if !timer_mode {
            match gaze_tick.status {
                crate::runtime::Status::Waiting => "Waiting for a game",
                crate::runtime::Status::CalibrationNeeded => {
                    "Calibration needed — choose Calibrate gaze"
                }
                crate::runtime::Status::QuickCheckNeeded => {
                    "Quick check needed — open it from the tray before play"
                }
                crate::runtime::Status::TrackingLost => "No face / tracking lost — pulses paused",
                crate::runtime::Status::Paused => {
                    "Paused — waiting for eligible play / League foreground"
                }
                crate::runtime::Status::Active => "Active — gaze coaching",
            }
        } else {
            match decision.status {
                Status::Active => "Active — timer every 10 seconds",
                Status::Paused => "Paused",
                Status::Waiting => "Waiting for a game / alive player / League foreground",
            }
        };
        let tooltip = format!(
            "LUCAS — {text}{}",
            if gaze.runtime.solo_armed() {
                " · Next game solo (no pulses)"
            } else {
                ""
            }
        );
        if tooltip != last_status {
            status_item.set_text(text);
            tray.set_tooltip(Some(&tooltip))?;
            last_status = tooltip;
        }
        // Sleeping is bounded; overlay has its own 16 ms foreground gate. The
        // preview additionally checks that same gate in its WM_TIMER handler.
        thread::sleep(Duration::from_millis(33));
    }
    drop(alignment);
    if let Some((_, mut overlay)) = overlay {
        overlay.shutdown()?;
    }
    drop(gaze);
    drop(worker);
    drop(tray);
    Ok(delete_requested)
}

fn primary_screen() -> Option<lucas_store::ScreenGeometry> {
    // SAFETY: output pointers reference initialized stack values. This queries
    // only monitor geometry/DPI, never game pixels or process memory.
    unsafe {
        let monitor = MonitorFromPoint(POINT { x: 0, y: 0 }, MONITOR_DEFAULTTOPRIMARY);
        let mut info = MONITORINFO {
            cbSize: std::mem::size_of::<MONITORINFO>() as u32,
            ..Default::default()
        };
        let (mut x, mut y) = (0, 0);
        if GetMonitorInfoW(monitor, &mut info) == 0
            || GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut x, &mut y) < 0
            || x == 0
        {
            return None;
        }
        Some(lucas_store::ScreenGeometry {
            left: info.rcMonitor.left,
            top: info.rcMonitor.top,
            width: (info.rcMonitor.right - info.rcMonitor.left) as u32,
            height: (info.rcMonitor.bottom - info.rcMonitor.top) as u32,
            scale_factor: x as f64 / 96.0,
        })
    }
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

type Poll = (Duration, Option<lucas_game::Snapshot>);
struct ApiWorker {
    state: Arc<Mutex<VecDeque<Poll>>>,
    generation: Arc<AtomicU64>,
    stop: Arc<AtomicBool>,
    handle: Option<thread::JoinHandle<()>>,
}
impl ApiWorker {
    fn start(origin: Instant) -> io::Result<Self> {
        let state = Arc::new(Mutex::new(VecDeque::new()));
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
                            // The cumulative event log rediscovers deaths after a tray stall.
                            if state.len() == 8 {
                                state.pop_front();
                            }
                            state.push_back((completed, result.and_then(std::result::Result::ok)));
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
    fn drain(&self) -> Vec<Poll> {
        self.state
            .lock()
            .map(|mut s| s.drain(..).collect())
            .unwrap_or_default()
    }
    fn set_enabled(&self, enabled: bool) {
        let old = self.generation.load(Ordering::Acquire);
        self.generation
            .store(((old / 2 + 1) * 2) | u64::from(enabled), Ordering::Release);
        if let Ok(mut state) = self.state.lock() {
            state.clear();
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
