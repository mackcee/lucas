//! CPU-only inference adapters. Verified bytes are passed directly to the runtime
//! to avoid a verify-then-reopen race. No model-fetching runtime feature is enabled.
use crate::Result;
#[cfg(feature = "ort")]
use crate::{Error, models::VerifiedModels};

pub struct DetectionOutput {
    pub boxes: Vec<f32>,
    pub scores: Vec<f32>,
}
pub struct MeshOutput {
    pub coordinates: Vec<f32>,
    pub presence_logit: f32,
}
/// Small seam for deterministic tracker tests and future CPU implementations.
pub trait InferenceBackend {
    fn detect(&mut self, input_nhwc: &[f32]) -> Result<DetectionOutput>;
    fn mesh(&mut self, input_nhwc: &[f32]) -> Result<MeshOutput>;
}

#[cfg(feature = "ort")]
pub struct OrtBackend {
    detector: ort::session::Session,
    mesh: ort::session::Session,
}
#[cfg(feature = "ort")]
impl OrtBackend {
    pub fn new(models: VerifiedModels) -> Result<Self> {
        // ort rc.14 permits only one environment per process, even after drop.
        static ENVIRONMENT: std::sync::OnceLock<Result<ort::environment::Environment>> =
            std::sync::OnceLock::new();
        let environment = ENVIRONMENT
            .get_or_init(|| {
                ort::init()
                    .with_name("lucas-gaze")
                    .with_telemetry(false)
                    .build()
                    .map_err(|_| Error::InvalidModel)
            })
            .as_ref()
            .map_err(Clone::clone)?;
        fn load(
            environment: &ort::environment::Environment,
            bytes: &[u8],
        ) -> Result<ort::session::Session> {
            // Explicit CPU EP, sequential execution, one compute thread, no busy
            // spin while idle. GPU is reserved for the game.
            ort::session::Session::builder(environment)
                .map_err(|_| Error::InvalidModel)?
                .with_execution_providers([ort::ep::CPU::default().build()])
                .map_err(|_| Error::InvalidModel)?
                .with_intra_threads(1)
                .map_err(|_| Error::InvalidModel)?
                .with_inter_threads(1)
                .map_err(|_| Error::InvalidModel)?
                .with_parallel_execution(false)
                .map_err(|_| Error::InvalidModel)?
                .with_config_entry("session.intra_op.allow_spinning", "0")
                .map_err(|_| Error::InvalidModel)?
                .with_config_entry("session.inter_op.allow_spinning", "0")
                .map_err(|_| Error::InvalidModel)?
                .commit_from_memory(bytes)
                .map_err(|_| Error::InvalidModel)
        }
        Ok(Self {
            detector: load(environment, &models.detector)?,
            mesh: load(environment, &models.landmarker)?,
        })
    }
}
#[cfg(feature = "ort")]
fn ort_run(
    session: &mut ort::session::Session,
    size: usize,
    input: &[f32],
    names: &[&str],
) -> Result<Vec<Vec<f32>>> {
    if input.len() != size * size * 3 {
        return Err(Error::InvalidFrame);
    }
    let tensor = ort::value::TensorRef::from_array_view(([1, size, size, 3], input))
        .map_err(|_| Error::InvalidFrame)?;
    let outputs = session
        .run(ort::inputs![tensor])
        .map_err(|_| Error::InferenceFailed)?;
    names
        .iter()
        .map(|name| {
            let output = outputs.get(*name).ok_or(Error::InvalidOutput)?;
            Ok(output
                .try_extract_tensor::<f32>()
                .map_err(|_| Error::InvalidOutput)?
                .1
                .to_vec())
        })
        .collect()
}
#[cfg(feature = "ort")]
impl InferenceBackend for OrtBackend {
    fn detect(&mut self, input: &[f32]) -> Result<DetectionOutput> {
        let mut outputs = ort_run(
            &mut self.detector,
            128,
            input,
            &["regressors", "classificators"],
        )?;
        let scores = outputs.pop().ok_or(Error::InvalidOutput)?;
        let boxes = outputs.pop().ok_or(Error::InvalidOutput)?;
        Ok(DetectionOutput { boxes, scores })
    }
    fn mesh(&mut self, input: &[f32]) -> Result<MeshOutput> {
        let mut outputs = ort_run(&mut self.mesh, 256, input, &["Identity", "Identity_1"])?;
        let score = outputs.pop().ok_or(Error::InvalidOutput)?;
        if score.len() != 1 {
            return Err(Error::InvalidOutput);
        }
        Ok(MeshOutput {
            coordinates: outputs.pop().ok_or(Error::InvalidOutput)?,
            presence_logit: score[0],
        })
    }
}
