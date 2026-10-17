//! Manual choices and resumable setup. No automatic promotion or scheduling.
use lucas_core::{Phase, TRAINING_LEVELS, TrainingLevel};
use lucas_store::{Settings, Store};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

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

pub fn level_picker(ui: &mut eframe::egui::Ui, index: &mut usize) {
    eframe::egui::ComboBox::from_label("Training level")
        .selected_text(level_label(TRAINING_LEVELS[*index]))
        .show_ui(ui, |ui| {
            for (i, level) in TRAINING_LEVELS.iter().enumerate() {
                ui.selectable_value(index, i, level_label(*level));
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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Setup {
    pub step: SetupStep,
    pub draft: Settings,
    pub borderless_skipped: bool,
    calibrated_camera: Option<u32>,
    calibration: Option<lucas_store::Calibration>,
}
const DRAFT_FILE: &str = "setup-draft.toml";
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
        let path = store.data_folder().join(DRAFT_FILE);
        match std::fs::read_to_string(path) {
            Ok(text) => {
                let mut setup: Self = toml::from_str(&text)?;
                setup.draft.validate()?;
                if let Some(profile) = &setup.calibration {
                    profile.validate()?;
                    let model =
                        lucas_gaze::calibration::CalibrationModel::from_bytes(&profile.model)?;
                    if !model
                        .validation()
                        .passes(model.target_false_positive_rate())
                    {
                        return Err("Draft calibration did not pass validation".into());
                    }
                }
                if !setup.calibration_passed() {
                    setup.calibrated_camera = None;
                    if setup.step.number() > SetupStep::Calibration.number() {
                        setup.step = SetupStep::Calibration;
                    }
                }
                Ok(setup)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(Self::new(store.load_settings()?))
            }
            Err(e) => Err(e.into()),
        }
    }
    pub fn checkpoint(&self, folder: &Path) -> crate::trace::Result<()> {
        self.draft.validate()?;
        let mut file = tempfile::NamedTempFile::new_in(folder)?;
        file.write_all(toml::to_string(self)?.as_bytes())?;
        file.as_file().sync_all()?;
        file.persist(folder.join(DRAFT_FILE))?;
        Ok(())
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
        self.checkpoint(store.data_folder())?;
        // Fail closed across the two durable files: a crash must never pair a
        // new camera with old weights. The draft keeps Finish retryable.
        match std::fs::remove_file(store.data_folder().join("calibration.json")) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
        store.save_settings(&self.draft)?;
        store.save_calibration(profile)?;
        match std::fs::remove_file(store.data_folder().join(DRAFT_FILE)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(e.into()),
        }
    }
}

pub fn needs_setup(folder: &Path) -> std::io::Result<bool> {
    Ok(!folder.join("settings.toml").try_exists()? || folder.join(DRAFT_FILE).try_exists()?)
}
