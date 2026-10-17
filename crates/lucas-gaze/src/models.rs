//! Hash-pinned, local-only model loading. The lock is compiled into the application,
//! not read from beside the models (where replacing both files could bypass it).
use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::{fs::File, io::Read, path::Path};

pub const DETECTOR: &str = "face_detector.onnx";
pub const LANDMARKER: &str = "face_landmarks_detector.onnx";
pub const LOCK: &str = include_str!("../../../models/SHA256SUMS");
const MAX_MODEL_BYTES: u64 = 16 * 1024 * 1024;

pub struct VerifiedModels {
    pub(crate) detector: Vec<u8>,
    pub(crate) landmarker: Vec<u8>,
}
impl VerifiedModels {
    pub fn load(directory: &Path) -> Result<Self> {
        Ok(Self {
            detector: read_verified(directory, DETECTOR)?,
            landmarker: read_verified(directory, LANDMARKER)?,
        })
    }
    pub fn total_bytes(&self) -> usize {
        self.detector.len() + self.landmarker.len()
    }
}
fn read_verified(directory: &Path, name: &str) -> Result<Vec<u8>> {
    let file = File::open(directory.join(name)).map_err(|_| Error::ModelReadFailed)?;
    let mut bytes = Vec::new();
    file.take(MAX_MODEL_BYTES + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| Error::ModelReadFailed)?;
    if bytes.len() as u64 > MAX_MODEL_BYTES {
        return Err(Error::ModelHashMismatch);
    }
    verify(name, &bytes)?;
    Ok(bytes)
}
fn verify(name: &str, bytes: &[u8]) -> Result<()> {
    let expected = LOCK
        .lines()
        .filter_map(|l| l.split_once("  "))
        .find(|(_, n)| *n == name)
        .map(|(hash, _)| hash)
        .ok_or(Error::InvalidModel)?;
    if format!("{:x}", Sha256::digest(bytes)) != expected {
        return Err(Error::ModelHashMismatch);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn tampered_missing_and_unknown_models_fail_closed() {
        assert_eq!(verify(DETECTOR, b"tampered"), Err(Error::ModelHashMismatch));
        assert_eq!(verify("unknown", b""), Err(Error::InvalidModel));
        assert!(matches!(
            VerifiedModels::load(Path::new("/nonexistent-lucas-models")),
            Err(Error::ModelReadFailed)
        ));
        assert!(
            VerifiedModels::load(&Path::new(env!("CARGO_MANIFEST_DIR")).join("../../models"))
                .unwrap()
                .total_bytes()
                > 1_000_000
        );
    }
}
