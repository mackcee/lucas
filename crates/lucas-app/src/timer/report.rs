//! Numeric geometry only. No screenshots, player identity, paths or game events.
use super::alignment_store::Key;
use lucas_geometry::{Alignment, GeometryError, MinimapSettings, Rect, minimap_rect};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Edges {
    pub left: f64,
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
}
impl From<Rect> for Edges {
    fn from(rect: Rect) -> Self {
        Self {
            left: rect.left(),
            top: rect.top(),
            right: rect.right(),
            bottom: rect.bottom(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GeometryReport {
    pub schema_version: u32,
    pub app_version: String,
    pub coordinate_space: String,
    pub game_client_rect: Edges,
    pub resolution: [u32; 2],
    pub minimap_scale: f64,
    pub flipped: bool,
    pub formula_rect: Edges,
    pub aligned_rect: Edges,
}

impl GeometryReport {
    pub fn new(
        client: Rect,
        settings: MinimapSettings,
        alignment: Alignment,
    ) -> Result<Self, GeometryError> {
        let key = Key::new(client, settings);
        Ok(Self {
            schema_version: 1,
            app_version: env!("CARGO_PKG_VERSION").into(),
            coordinate_space: "physical desktop pixels; half-open edges".into(),
            game_client_rect: client.into(),
            resolution: [key.width, key.height],
            minimap_scale: settings.minimap_scale,
            flipped: settings.flipped,
            formula_rect: minimap_rect(client, settings, Alignment::default())?.into(),
            aligned_rect: minimap_rect(client, settings, alignment)?.into(),
        })
    }

    pub fn save(&self, data_folder: &Path) -> io::Result<PathBuf> {
        let folder = data_folder.join("reports");
        fs::create_dir_all(&folder)?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?
            .as_millis();
        let bytes = serde_json::to_vec_pretty(self).map_err(io::Error::other)?;
        // Never overwrite a report, even on rapid repeated clicks or clock rollback.
        for suffix in 0..1000 {
            let path = folder.join(format!("minimap-{stamp}-{suffix}.json"));
            match fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&path)
            {
                Ok(mut file) => {
                    file.write_all(&bytes)?;
                    file.write_all(b"\n")?;
                    file.sync_all()?;
                    return Ok(path);
                }
                Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
                Err(e) => return Err(e),
            }
        }
        Err(io::Error::new(
            io::ErrorKind::AlreadyExists,
            "too many reports at this timestamp",
        ))
    }
}
