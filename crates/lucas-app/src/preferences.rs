//! Manual choices and in-memory setup. No automatic promotion or scheduling.
use lucas_core::{Phase, TRAINING_LEVELS, TrainingLevel};
use lucas_store::{Settings, Store};

pub fn level_label(level: TrainingLevel) -> String {
    let seconds = level.pulse_delay_ms.map(|ms| ms as f64 / 1000.0);
    match level.phase {
        Phase::Baseline => "Baseline (no pulses)".into(),
        Phase::Ramp => format!(
            "{} s{}",
            seconds.unwrap_or_default(),
            if Some(level)
                == TRAINING_LEVELS
                    .iter()
                    .find(|l| l.phase == Phase::Ramp)
                    .copied()
            {
                " (start)"
            } else {
                ""
            }
        ),
        Phase::Fade => format!("Fade: {} s", seconds.unwrap_or_default()),
        Phase::Graduated => format!(
            "Off (keep the {} s goal)",
            level.standard_ms as f64 / 1000.0
        ),
    }
}

pub fn selectable_levels() -> impl Iterator<Item = (usize, TrainingLevel)> {
    TRAINING_LEVELS
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, level)| level.phase != Phase::Baseline)
}

pub fn level_picker(ui: &mut eframe::egui::Ui, index: &mut usize) {
    eframe::egui::ComboBox::from_label("Training level")
        .selected_text(level_label(TRAINING_LEVELS[*index]))
        .show_ui(ui, |ui| {
            for (i, level) in selectable_levels() {
                ui.selectable_value(index, i, level_label(level));
            }
        });
    let level = TRAINING_LEVELS[*index];
    ui.label(format!(
        "Goal: a minimap check every {} s. Changes apply next game.",
        level.standard_ms as f64 / 1000.0
    ));
    if level.phase == Phase::Fade {
        ui.label(format!(
            "Help: {:?} · {:.0}% of pulse opportunities. The goal stays the same.",
            level.salience,
            level.pulse_probability * 100.0
        ));
    }
}

/// Shared by standalone, tray and setup calibration. Presets follow the actual
/// calibration monitor's pixel aspect ratio, not its Windows scaling setting.
pub fn calibration_picker(
    ui: &mut eframe::egui::Ui,
    preferences: &mut lucas_store::CalibrationPreferences,
    resolution: [u32; 2],
) {
    use eframe::egui::{ComboBox, DragValue};
    use lucas_store::ScreenSize;
    let label = |size| match size {
        ScreenSize::Inches24 => "24 in",
        ScreenSize::Inches27 => "27 in",
        ScreenSize::Inches32 => "32 in",
        ScreenSize::Inches34 => "34 in",
        ScreenSize::Custom => "Custom",
    };
    ComboBox::from_label("Screen size (diagonal)")
        .selected_text(label(preferences.screen_size))
        .show_ui(ui, |ui| {
            for size in [
                ScreenSize::Inches24,
                ScreenSize::Inches27,
                ScreenSize::Inches32,
                ScreenSize::Inches34,
                ScreenSize::Custom,
            ] {
                ui.selectable_value(&mut preferences.screen_size, size, label(size));
            }
        });
    if preferences.screen_size == ScreenSize::Custom {
        ui.horizontal_wrapped(|ui| {
            for (label, value) in ["Screen width (mm)", "Height (mm)"]
                .into_iter()
                .zip(&mut preferences.custom_size_mm)
            {
                ui.horizontal(|ui| {
                    ui.label(label);
                    ui.add(DragValue::new(value).range(100..=3000));
                });
            }
        });
    } else if let Ok([width, height]) = preferences.physical_size_mm(resolution) {
        ui.label(format!(
            "{width} × {height} mm · based on the main monitor's {} × {} resolution",
            resolution[0], resolution[1]
        ));
    }
    ui.horizontal(|ui| {
        ui.label("Assumed distance (mm)");
        ui.add(DragValue::new(&mut preferences.viewing_distance_mm).range(100..=3000));
    });
    ui.label("Screen size and distance are remembered when you start calibration. Choose Custom for measured dimensions, not Windows scaling.");
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum SetupStep {
    #[default]
    Intro,
    Camera,
    Borderless,
    Calibration,
    Alignment,
    Level,
}
impl SetupStep {
    pub fn number(self) -> usize {
        match self {
            Self::Intro => 1,
            Self::Camera => 2,
            Self::Borderless => 3,
            Self::Calibration => 4,
            Self::Alignment => 5,
            Self::Level => 6,
        }
    }
    pub fn title(self) -> &'static str {
        match self {
            Self::Intro => "Build your map-check habit",
            Self::Camera => "Choose your camera",
            Self::Borderless => "Use League in Borderless",
            Self::Calibration => "Calibrate your gaze",
            Self::Alignment => "Align the minimap in your next game",
            Self::Level => "Choose your starting level",
        }
    }
}

/// Draft settings, calibration weights and wizard position, never samples.
#[derive(Clone, Debug)]
pub struct Setup {
    pub step: SetupStep,
    pub draft: Settings,
    pub borderless_skipped: bool,
    calibrated_camera: Option<u32>,
    calibration: Option<lucas_store::Calibration>,
}
impl Setup {
    pub fn new(settings: Settings) -> Self {
        Self {
            step: SetupStep::Intro,
            draft: settings,
            borderless_skipped: false,
            calibrated_camera: None,
            calibration: None,
        }
    }
    pub fn load(store: &Store) -> crate::trace::Result<Self> {
        Ok(Self::new(store.load_settings()?))
    }
    pub fn select_camera(&mut self, index: u32) {
        if self.draft.camera_index != index {
            self.begin_calibration();
        }
        self.draft.camera_index = index;
    }
    pub fn begin_calibration(&mut self) {
        self.calibrated_camera = None;
        self.calibration = None;
    }
    pub fn calibrated(&mut self, profile: lucas_store::Calibration) {
        self.calibration = Some(profile);
        self.calibrated_camera = Some(self.draft.camera_index);
    }
    pub fn calibration_passed(&self) -> bool {
        self.calibration.is_some() && self.calibrated_camera == Some(self.draft.camera_index)
    }
    pub fn advance(&mut self) -> bool {
        self.step = match self.step {
            SetupStep::Intro => SetupStep::Camera,
            SetupStep::Camera => SetupStep::Borderless,
            SetupStep::Borderless => SetupStep::Calibration,
            SetupStep::Calibration if self.calibration_passed() => SetupStep::Alignment,
            SetupStep::Alignment => SetupStep::Level,
            _ => return false,
        };
        true
    }
    pub fn skip_borderless(&mut self) {
        if self.step == SetupStep::Borderless {
            self.borderless_skipped = true;
            self.advance();
        }
    }
    pub fn back(&mut self) {
        self.step = match self.step {
            SetupStep::Intro | SetupStep::Camera => SetupStep::Intro,
            SetupStep::Borderless => SetupStep::Camera,
            SetupStep::Calibration => SetupStep::Borderless,
            SetupStep::Alignment => SetupStep::Calibration,
            SetupStep::Level => SetupStep::Alignment,
        };
    }
    pub fn finish(&self, store: &Store) -> crate::trace::Result<()> {
        if self.step != SetupStep::Level || !self.calibration_passed() {
            return Err("Complete camera calibration before finishing setup".into());
        }
        let profile = self.calibration.as_ref().ok_or("Missing calibration")?;
        profile.validate()?;
        let mut settings = self.draft.clone();
        // Calibration remembers its display choices at start, independently of
        // this draft. Finishing setup must not restore the older defaults.
        settings.calibration = store.load_settings()?.calibration;
        settings.setup_completed = false;
        settings.validate()?;
        match std::fs::remove_file(store.data_folder().join("calibration.json")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        store.save_settings(&settings)?;
        store.save_calibration(profile)?;
        settings.setup_completed = true;
        store.save_settings(&settings)?;
        Ok(())
    }
}

pub fn needs_setup(settings: &Settings) -> bool {
    !settings.setup_completed
}
