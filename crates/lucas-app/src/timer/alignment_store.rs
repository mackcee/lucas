//! Temporary application-local persistence, to be absorbed by lucas-store.
//! Only alignment numbers live here; no game data, images or gaze samples.
use lucas_geometry::{Alignment, MinimapSettings, Rect};
use serde::{Deserialize, Serialize};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Key {
    pub width: u32,
    pub height: u32,
    pub minimap_scale: f64,
    pub flipped: bool,
}

impl Key {
    pub fn new(client: Rect, settings: MinimapSettings) -> Self {
        Self {
            width: client.width().round() as u32,
            height: client.height().round() as u32,
            minimap_scale: settings.minimap_scale,
            flipped: settings.flipped,
        }
    }

    fn valid(self) -> bool {
        self.width > 0
            && self.height > 0
            && self.minimap_scale.is_finite()
            && (0.0..=3.0).contains(&self.minimap_scale)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Entry {
    key: Key,
    offset_x_px: f64,
    offset_y_px: f64,
    scale_multiplier: f64,
}

impl Entry {
    fn alignment(&self) -> Alignment {
        Alignment {
            offset_x_px: self.offset_x_px,
            offset_y_px: self.offset_y_px,
            scale_multiplier: self.scale_multiplier,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AlignmentStore {
    version: u32,
    #[serde(default)]
    profiles: Vec<Entry>,
}

impl Default for AlignmentStore {
    fn default() -> Self {
        Self {
            version: 1,
            profiles: Vec::new(),
        }
    }
}

pub fn data_folder() -> io::Result<PathBuf> {
    std::env::var_os("LOCALAPPDATA")
        .filter(|s| !s.is_empty())
        .map(|root| PathBuf::from(root).join("lucas"))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "LOCALAPPDATA is not set"))
}

impl AlignmentStore {
    pub fn load(folder: &Path) -> io::Result<Self> {
        match fs::read_to_string(folder.join("alignment.toml")) {
            Ok(text) => Self::decode(&text),
            Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Self::default()),
            Err(e) => Err(e),
        }
    }

    pub fn decode(text: &str) -> io::Result<Self> {
        let store: Self = toml::from_str(text).map_err(io::Error::other)?;
        if store.version != 1 {
            return Err(io::Error::other("unsupported alignment file version"));
        }
        for (i, entry) in store.profiles.iter().enumerate() {
            validate(entry.key, entry.alignment())?;
            if store.profiles[..i].iter().any(|e| e.key == entry.key) {
                return Err(io::Error::other("duplicate alignment profile"));
            }
        }
        Ok(store)
    }

    pub fn get(&self, key: Key) -> Alignment {
        self.profiles
            .iter()
            .find(|entry| entry.key == key)
            .map(Entry::alignment)
            .unwrap_or_default()
    }

    pub fn set(&mut self, key: Key, alignment: Alignment) -> io::Result<()> {
        validate(key, alignment)?;
        let entry = Entry {
            key,
            offset_x_px: alignment.offset_x_px,
            offset_y_px: alignment.offset_y_px,
            scale_multiplier: alignment.scale_multiplier,
        };
        if let Some(old) = self.profiles.iter_mut().find(|e| e.key == key) {
            *old = entry;
        } else {
            self.profiles.push(entry);
        }
        Ok(())
    }

    /// Write/sync a sibling first, then replace. A failed write leaves the previous
    /// file intact. Windows rename does not replace, so use MoveFileExW there.
    pub fn save(&self, folder: &Path) -> io::Result<()> {
        fs::create_dir_all(folder)?;
        let text = toml::to_string_pretty(self).map_err(io::Error::other)?;
        let temporary = folder.join("alignment.toml.tmp");
        let destination = folder.join("alignment.toml");
        use io::Write;
        let mut file = fs::File::create(&temporary)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        drop(file);
        replace(&temporary, &destination)
    }
}

fn validate(key: Key, alignment: Alignment) -> io::Result<()> {
    if !key.valid()
        || !alignment.offset_x_px.is_finite()
        || !alignment.offset_y_px.is_finite()
        || !alignment.scale_multiplier.is_finite()
        || !(0.25..=4.0).contains(&alignment.scale_multiplier)
        || alignment.offset_x_px.abs() > key.width as f64
        || alignment.offset_y_px.abs() > key.height as f64
    {
        return Err(io::Error::other("alignment values out of range"));
    }
    Ok(())
}

#[cfg(not(windows))]
fn replace(from: &Path, to: &Path) -> io::Result<()> {
    fs::rename(from, to)
}

#[cfg(windows)]
fn replace(from: &Path, to: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH, MoveFileExW,
    };
    let from: Vec<_> = from.as_os_str().encode_wide().chain(Some(0)).collect();
    let to: Vec<_> = to.as_os_str().encode_wide().chain(Some(0)).collect();
    // SAFETY: live NUL-terminated paths; neither buffer is retained by Windows.
    if unsafe {
        MoveFileExW(
            from.as_ptr(),
            to.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    } == 0
    {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}
