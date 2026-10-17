//! Post-game only. The caller owns game-end detection and must never open this
//! window during play. `PostGameCard` can also be hosted by an existing eframe app.

use eframe::egui::{self, Color32, RichText};
use egui_plot::{Legend, Line, MarkerShape, Plot, Points, VLine};
use lucas_core::{DeathClassification, Phase, Salience, TrainingLevel};
use lucas_store::{GameSummary, ProbePoint, Store, StoredEventKind};
use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub const DELETE_REQUESTED_EXIT_CODE: u8 = 42;

const AMBER: Color32 = Color32::from_rgb(255, 191, 96);
const SEEN: Color32 = Color32::from_rgb(118, 211, 205);
const UNKNOWN: Color32 = Color32::from_rgb(190, 193, 202);

pub struct PostGameCard {
    summary: Option<GameSummary>,
    probes: Vec<ProbePoint>,
    store: Option<Store>,
    data_folder: Option<PathBuf>,
    demo: bool,
    confirm_delete: bool,
    delete_request: Option<Arc<AtomicBool>>,
    notice: Option<String>,
}

impl PostGameCard {
    /// Transfers the sole database handle to the card so deletion can close it.
    /// Stop trace/report writers and release other handles before calling this.
    pub fn from_store(store: Store, game_id: &str) -> lucas_store::Result<Self> {
        let summary = store
            .summary(game_id)?
            .ok_or(lucas_store::StoreError::Invalid("game not found"))?;
        let probes = store.probe_trend()?;
        Ok(Self {
            summary: Some(summary),
            probes,
            data_folder: Some(store.data_folder().to_path_buf()),
            store: Some(store),
            demo: false,
            confirm_delete: false,
            delete_request: None,
            notice: None,
        })
    }

    pub fn demo() -> Self {
        Self {
            summary: Some(demo_summary()),
            probes: demo_probes(),
            store: None,
            data_folder: None,
            demo: true,
            confirm_delete: false,
            delete_request: None,
            notice: None,
        }
    }

    fn delete_data(&mut self) {
        if let Some(request) = &self.delete_request {
            self.store = None;
            request.store(true, Ordering::Release);
            return;
        }
        let Some(directory) = &self.data_folder else {
            return;
        };
        let result = self
            .store
            .take()
            .map_or_else(|| Store::open_in(directory), Ok)
            .and_then(Store::delete_all_data);
        match result {
            Ok(()) => {
                self.summary = None;
                self.probes.clear();
                self.notice = None;
            }
            Err(error) => {
                self.notice = Some(format!(
                    "Deletion was incomplete: {error}. Close other LUCAS windows, then choose Delete all data again to retry."
                ));
            }
        }
    }

    /// The sole presentation function: no game API, camera access, or live score.
    pub fn show(&mut self, ui: &mut egui::Ui) {
        egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
            ui.set_max_width(ui.available_width().min(820.0));
            ui.heading(RichText::new("Your map awareness").strong());
            if self.demo {
                ui.label(RichText::new("Demo · synthetic game, not saved").color(AMBER));
            }
            ui.add_space(8.0);
            if let Some(summary) = &self.summary {
                render_summary(ui, summary, &self.probes);
            } else {
                ui.label("Local data deleted. Settings and calibration will start fresh next time.");
            }
            ui.add_space(24.0);
            ui.separator();
            ui.label("Stored on this computer only. Camera frames are never saved.");
            ui.horizontal_wrapped(|ui| {
                if ui.add_enabled(self.data_folder.is_some(), egui::Button::new("Open data folder")).clicked()
                    && let Some(path) = &self.data_folder
                    && let Err(error) = open_data_folder(path)
                {
                    self.notice = Some(format!("Could not open the folder: {error}. You can copy the path below."));
                }
                if ui.add_enabled(self.data_folder.is_some(), egui::Button::new("Delete all data…")).clicked() {
                    self.confirm_delete = true;
                }
            });
            if let Some(path) = &self.data_folder {
                ui.label(path.display().to_string());
            }
            if self.demo {
                ui.small("Data controls are disabled in demo mode; no real data is opened.");
            }
            if self.confirm_delete {
                ui.add_space(8.0);
                ui.label("Permanently delete settings, calibration, all game summaries, traces and reports? This cannot be undone.");
                if self.delete_request.is_some() {
                    ui.label("LUCAS will quit before deleting data. Restart it to begin with fresh settings and calibration.");
                }
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() { self.confirm_delete = false; }
                    if ui.button("Permanently delete all data").clicked() {
                        self.confirm_delete = false;
                        self.delete_data();
                    }
                });
            }
            if let Some(notice) = &self.notice { ui.label(RichText::new(notice).color(AMBER)); }
        });
        if self
            .delete_request
            .as_ref()
            .is_some_and(|r| r.load(Ordering::Acquire))
        {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

pub fn show_runtime_postgame(game_id: &str) -> Result<bool, String> {
    let request = Arc::new(AtomicBool::new(false));
    let store = Store::open().map_err(|e| e.to_string())?;
    let mut card = PostGameCard::from_store(store, game_id).map_err(|e| e.to_string())?;
    card.delete_request = Some(Arc::clone(&request));
    show_postgame(card).map_err(|e| e.to_string())?;
    Ok(request.load(Ordering::Acquire))
}

impl eframe::App for PostGameCard {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::Frame::new()
            .inner_margin(24.0)
            .show(ui, |ui| self.show(ui));
    }
}

/// Standalone native window, called on the UI/main thread after a game ends.
/// An existing UI host should embed `PostGameCard::show` instead of starting a
/// second event loop. Wiring and in-game suppression belong to the lifecycle.
pub fn show_postgame(card: PostGameCard) -> eframe::Result {
    eframe::run_native(
        "LUCAS · Post-game",
        eframe::NativeOptions {
            viewport: egui::ViewportBuilder::default()
                .with_inner_size([860.0, 880.0])
                .with_min_inner_size([480.0, 420.0]),
            ..Default::default()
        },
        Box::new(|cc| {
            cc.egui_ctx.set_visuals(egui::Visuals::dark());
            cc.egui_ctx.all_styles_mut(|style| {
                style.spacing.item_spacing = egui::vec2(12.0, 8.0);
                style.spacing.interact_size.y = 28.0;
                style
                    .text_styles
                    .insert(egui::TextStyle::Body, egui::FontId::proportional(16.0));
                style
                    .text_styles
                    .insert(egui::TextStyle::Heading, egui::FontId::proportional(25.0));
            });
            Ok(Box::new(card))
        }),
    )
}

fn section(ui: &mut egui::Ui, title: &str) {
    ui.add_space(18.0);
    ui.label(RichText::new(title).size(19.0).strong());
}

fn render_summary(ui: &mut egui::Ui, summary: &GameSummary, probes: &[ProbePoint]) {
    let m = &summary.metrics;
    ui.label(format!(
        "{} · {} · {} · {}",
        summary
            .context
            .champion
            .as_deref()
            .unwrap_or("Champion unknown"),
        summary.context.role.as_deref().unwrap_or("Role unknown"),
        game_clock(summary.context.length_ms),
        if summary.context.probe_game {
            "Solo probe"
        } else {
            "Assisted game"
        }
    ));
    section(ui, "Time within your map-check goal");
    ui.label(
        RichText::new(format!(
            "C: {} · goal {} s",
            percent(m.compliance),
            summary.initial_level.standard_ms as f64 / 1000.0
        ))
        .size(23.0)
        .strong(),
    );
    ui.label(format!("Share of eligible play not spent beyond the goal gap. Based on {} of tracked, active play; loading, death and tracking loss are excluded.", game_clock(m.eligible_ms)));
    if summary.standard_changed {
        ui.label("The goal changed during this game. C uses the goal active at each moment; this game is not joined to a fixed-goal trend.");
    }
    section(ui, "Blind deaths");
    if summary.context.deaths_available {
        let total = m.blind_deaths + m.seen_deaths + m.unknown_deaths;
        ui.label(
            RichText::new(format!(
                "{} / {} blind · {} seen · {} unknown",
                m.blind_deaths, total, m.seen_deaths, m.unknown_deaths
            ))
            .size(20.0)
            .strong(),
        );
        ui.label(format!("Blind means no minimap evidence for at least {} s. Unknown means insufficient tracking; unknown deaths are not counted as blind. This is correlation, not a cause of death.", summary.blind_death_threshold_ms as f64 / 1000.0));
        if total == 0 {
            ui.label("No deaths recorded in this game.");
        }
        for (index, event) in summary.events.iter().enumerate() {
            if let StoredEventKind::Death {
                classification,
                seconds_blind,
                tracked_fraction,
            } = event.kind
            {
                let (name, colour) = death_style(classification);
                let gap = seconds_blind.map_or_else(
                    || "No earlier minimap evidence".into(),
                    |seconds| format!("{seconds:.1} s elapsed gap"),
                );
                let uncertain = if classification == DeathClassification::Unknown {
                    " (uncertain)"
                } else {
                    ""
                };
                egui::CollapsingHeader::new(
                    RichText::new(format!(
                        "{} · {name} · {gap}{uncertain}",
                        game_clock(event.game_time_ms)
                    ))
                    .color(colour)
                    .size(16.0),
                )
                .id_salt(("death-details", index))
                .show(ui, |ui| {
                    ui.small(format!(
                        "{:.0}% of the classification window tracked",
                        tracked_fraction * 100.0
                    ));
                    death_timeline(ui, summary, index, event.game_time_ms, colour);
                });
            }
        }
    } else {
        ui.label("Death data unavailable in window-only mode; blind deaths cannot be assessed.");
    }
    section(ui, "Check gaps");
    ui.horizontal_wrapped(|ui| {
        for (label, value) in [
            ("Median", m.gaps.median_ms),
            ("80th percentile", m.gaps.p80_ms),
            ("90th percentile", m.gaps.p90_ms),
            ("Longest", m.gaps.max_ms.map(|v| v as f64)),
        ] {
            ui.label(format!("{label}: {}", seconds(value)));
        }
    });
    ui.label(format!(
        "{} checks · {} checks/min · {} pulses · {} unprompted",
        m.checks,
        number(m.checks_per_minute),
        m.pulses,
        percent(m.unprompted_ratio)
    ));
    ui.small("Gaps count eligible time only, including the initial and unfinished final gap.");
    section(ui, "Tracking quality");
    ui.label(format!(
        "{} valid frames · calibration validation error: {}",
        percent(summary.context.tracking.valid_fraction()),
        summary
            .context
            .tracking
            .calibration_error_degrees
            .map_or_else(|| "not available".into(), |v| format!("{v:.1}°"))
    ));
    if summary
        .context
        .tracking
        .valid_fraction()
        .is_none_or(|v| v < 0.85)
    {
        ui.label(RichText::new("Tracking was limited. Treat this summary cautiously; missing tracking is not evidence of a missed check.").color(AMBER));
    }
    section(ui, "Solo-probe trend");
    probe_trend(ui, probes);
    section(ui, "Training wheels");
    ui.label(RichText::new(wheels_text(summary.wheels)).strong());
    if summary.context.probe_game {
        ui.label("Assistance was off for this solo probe.");
    }
    ui.label(next_criterion_text(summary.wheels));
    ui.small("Levels are changed manually in this MVP. This target is guidance, not an automatic promotion or a progress counter.");
}

fn death_timeline(
    ui: &mut egui::Ui,
    summary: &GameSummary,
    index: usize,
    death_ms: u64,
    colour: Color32,
) {
    let window = 20.0_f64
        .max(summary.blind_death_threshold_ms as f64 / 1000.0)
        .min(120.0);
    let mut checks = Vec::new();
    let mut pulses = Vec::new();
    for event in &summary.events {
        let offset = (event.game_time_ms as f64 - death_ms as f64) / 1000.0;
        if !(-window..=0.0).contains(&offset) {
            continue;
        }
        match event.kind {
            StoredEventKind::Check => checks.push([offset, 0.6]),
            StoredEventKind::Pulse => pulses.push([offset, 0.2]),
            _ => {}
        }
    }
    Plot::new(("death", index))
        .height(95.0)
        .allow_drag(false)
        .allow_zoom(false)
        .allow_scroll(false)
        .show_axes([true, false])
        .show_grid([true, false])
        .include_x(-window)
        .include_x(0.0)
        .include_y(0.0)
        .include_y(1.0)
        .x_axis_label("Game seconds · circles = checks · diamonds = pulses")
        .show(ui, |plot| {
            plot.points(Points::new("Check", checks).color(SEEN).radius(4.0_f32));
            plot.points(
                Points::new("Pulse", pulses)
                    .color(AMBER)
                    .shape(MarkerShape::Diamond)
                    .radius(4.0_f32),
            );
            plot.vline(VLine::new("Death", 0.0).color(colour));
        });
}

fn probe_trend(ui: &mut egui::Ui, probes: &[ProbePoint]) {
    if probes.len() < 2 {
        ui.label("Your trend appears after more solo-probe games (at least two).");
        return;
    }
    let measured = probes
        .iter()
        .filter(|p| p.compliance.is_some() && !p.standard_changed)
        .count();
    if measured < 2 {
        ui.label("More solo probes with a measurable, unchanged goal are needed for a trend.");
        return;
    }
    Plot::new("probe-trend")
        .height(160.0)
        .allow_scroll(false)
        .allow_drag(false)
        .allow_zoom(false)
        .include_y(0.0)
        .include_y(100.0)
        .x_axis_label("Solo probe, oldest to newest")
        .y_axis_label("C (%)")
        .legend(Legend::default())
        .show(ui, |plot| {
            // Separate standards: tightening S must not look like lost progress.
            for (standard, colour) in [
                (10_000, AMBER),
                (8_000, SEEN),
                (6_000, Color32::from_rgb(199, 176, 239)),
                (5_000, Color32::from_rgb(142, 192, 244)),
            ] {
                let points: Vec<[f64; 2]> = probes
                    .iter()
                    .enumerate()
                    .filter_map(|(index, p)| {
                        (p.standard_ms == standard && !p.standard_changed)
                            .then(|| p.compliance.map(|c| [index as f64 + 1.0, c * 100.0]))
                            .flatten()
                    })
                    .collect();
                for (index, pair) in probes.windows(2).enumerate() {
                    if pair
                        .iter()
                        .all(|p| p.standard_ms == standard && !p.standard_changed)
                        && let (Some(a), Some(b)) = (pair[0].compliance, pair[1].compliance)
                    {
                        plot.line(
                            Line::new(
                                format!("{} s goal", standard / 1000),
                                vec![
                                    [index as f64 + 1.0, a * 100.0],
                                    [index as f64 + 2.0, b * 100.0],
                                ],
                            )
                            .color(colour),
                        );
                    }
                }
                if !points.is_empty() {
                    plot.points(
                        Points::new(format!("{} s goal", standard / 1000), points)
                            .color(colour)
                            .radius(4.0_f32),
                    );
                }
            }
        });
    ui.small("Raw probe C; lines connect only games with the same goal. Missing scores and mid-game goal changes are not joined. This is not a promotion assessment.");
    if probes
        .iter()
        .any(|p| p.valid_fraction.is_none_or(|v| v < 0.85))
    {
        ui.label("Some probes had limited tracking. Their C may not represent the whole game.");
    }
}

pub fn wheels_text(level: TrainingLevel) -> String {
    let Some(delay) = level.pulse_delay_ms else {
        return "Wheels: off".into();
    };
    let salience = match level.salience {
        Salience::Strong => "strong",
        Salience::Medium => "medium",
        Salience::Soft => "soft",
        Salience::Minimal => "minimal",
    };
    format!(
        "Wheels: {} s · {salience} · {:.0}%",
        delay as f64 / 1000.0,
        level.pulse_probability * 100.0
    )
}

/// Describes the planned target from core's phase/level, not invented progress.
/// The actual promotion evaluator and scheduling remain outside this MVP.
pub fn next_criterion_text(level: TrainingLevel) -> String {
    match level.phase {
        Phase::Baseline => "Next target: complete two unassisted baseline games before choosing your starting goal.".into(),
        Phase::Ramp => format!("Next target: two games with no more than one in four gaps prompted and at least 85% unprompted checks, plus a passing solo probe at the {} s goal. Count only games with at least 12 minutes of eligible play and 85% valid tracking.", level.standard_ms as f64 / 1000.0),
        Phase::Fade => {
            let late = matches!(level.salience, Salience::Soft | Salience::Minimal);
            let threshold = if late { 80 } else { 75 };
            let minimum = if late { "three games across at least two days" } else { "two games" };
            format!("Next target: at least {threshold}% C and 85% unprompted checks in each of the last two assisted games, after {minimum} at this level. From medium wheels onward, the latest solo-probe C must be within 10 percentage points of assisted C. Count only games with at least 12 minutes of eligible play and 85% valid tracking. To turn wheels off, demonstrate at least 80% C on three consecutive solo probes on three different days, including a different role or champion.")
        },
        Phase::Graduated => "Wheels are off. Use occasional solo probes to check that your 5 s habit holds without assistance.".into(),
    }
}

fn death_style(class: DeathClassification) -> (&'static str, Color32) {
    match class {
        DeathClassification::Blind => ("Blind", AMBER),
        DeathClassification::Seen => ("Seen", SEEN),
        DeathClassification::Unknown => ("Unknown", UNKNOWN),
    }
}
fn percent(value: Option<f64>) -> String {
    value.map_or_else(|| "not available".into(), |v| format!("{:.0}%", v * 100.0))
}
fn number(value: Option<f64>) -> String {
    value.map_or_else(|| "not available".into(), |v| format!("{v:.1}"))
}
fn seconds(value: Option<f64>) -> String {
    value.map_or_else(
        || "not available".into(),
        |v| format!("{:.1} s", v / 1000.0),
    )
}
fn game_clock(ms: u64) -> String {
    format!("{}:{:02}", ms / 60_000, ms / 1000 % 60)
}

#[cfg(windows)]
fn open_data_folder(path: &Path) -> std::io::Result<()> {
    std::process::Command::new("explorer.exe")
        .arg(path)
        .spawn()?;
    Ok(())
}
#[cfg(not(windows))]
fn open_data_folder(_path: &Path) -> std::io::Result<()> {
    Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "folder opening is supported on Windows",
    ))
}

/// Developer CLI adapter. `demo` never opens the real data directory. A file
/// argument must be the store's games.sqlite3; a directory is accepted too.
pub fn run_dev(source: &std::ffi::OsStr) -> Result<(), Box<dyn std::error::Error>> {
    let card = if source == "demo" {
        PostGameCard::demo()
    } else {
        let path = Path::new(source);
        let directory = if path.is_file() {
            if path.file_name() != Some(std::ffi::OsStr::new(lucas_store::DATABASE_NAME)) {
                return Err("Expected a games.sqlite3 database or its data directory".into());
            }
            path.parent()
                .filter(|parent| !parent.as_os_str().is_empty())
                .unwrap_or(Path::new("."))
        } else {
            path
        };
        if !directory.join(lucas_store::DATABASE_NAME).is_file() {
            return Err(
                "No games.sqlite3 found at this path; use 'demo' for a synthetic game".into(),
            );
        }
        let store = Store::open_in(directory)?;
        let latest = store
            .latest_summary()?
            .ok_or("No stored games yet; use 'demo' for a synthetic game")?;
        PostGameCard::from_store(store, &latest.id)?
    };
    show_postgame(card).map_err(|error| error.to_string().into())
}

#[cfg(test)]
mod tests;

mod demo;
use demo::demo_probes;
pub use demo::demo_summary;
