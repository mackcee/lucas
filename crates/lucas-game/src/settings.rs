//! File discovery only. Feed each candidate's text to lucas-geometry's parser;
//! try the next candidate if reading or parsing fails. No duplicate HUD parser.

use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsFormat {
    PersistedJson,
    GameCfg,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SettingsFile {
    pub path: PathBuf,
    pub format: SettingsFormat,
}

impl SettingsFile {
    pub fn read(&self) -> io::Result<String> {
        fs::read_to_string(&self.path)
    }
}

/// User-specified installation roots take precedence. Each root contains Config.
/// Discovery is read-only; no registry, lockfile, LCU or game-process access.
#[derive(Clone, Debug, Default)]
pub struct SettingsLocator {
    roots: Vec<PathBuf>,
}

impl SettingsLocator {
    pub fn new(roots: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut result = Self::default();
        for root in roots {
            result.add_root(root);
        }
        result
    }

    pub fn add_root(&mut self, root: PathBuf) {
        if !self.roots.contains(&root) {
            self.roots.push(root);
        }
    }

    /// Add custom installs from Riot's local installation manifest. This file is
    /// ordinary installer metadata, not the League Client API. Unknown keys are
    /// ignored; malformed JSON is returned to the caller for diagnostics.
    pub fn add_install_manifest(&mut self, path: &Path) -> io::Result<()> {
        let text = fs::read_to_string(path)?;
        let value: serde_json::Value = serde_json::from_str(&text).map_err(io::Error::other)?;
        if let Some(installs) = value.get("associated_client").and_then(|v| v.as_object()) {
            for root in installs.keys() {
                let root = PathBuf::from(root.replace('\\', "/"));
                if root.join("LeagueClient.exe").is_file() {
                    self.add_root(root);
                }
            }
        }
        Ok(())
    }

    /// Returns ordered paths even if absent. Read errors remain visible to the
    /// adapter; existence is deliberately not raced with a separate metadata call.
    pub fn candidates(&self) -> Vec<SettingsFile> {
        self.roots
            .iter()
            .flat_map(|root| {
                [
                    SettingsFile {
                        path: root.join("Config/PersistedSettings.json"),
                        format: SettingsFormat::PersistedJson,
                    },
                    SettingsFile {
                        path: root.join("Config/game.cfg"),
                        format: SettingsFormat::GameCfg,
                    },
                ]
            })
            .collect()
    }

    /// Common Windows install roots plus the Riot installer manifest. An explicit
    /// path comes first; the app can expose it when discovery cannot find an install.
    #[cfg(windows)]
    pub fn windows_defaults(explicit: Option<PathBuf>) -> Self {
        use std::env;
        let mut locator = Self::new(explicit);
        if let Some(program_data) = env::var_os("ProgramData") {
            // Missing/stale manifest must not prevent trying standard paths.
            let _ = locator.add_install_manifest(
                &PathBuf::from(program_data).join("Riot Games/RiotClientInstalls.json"),
            );
        }
        let drive = env::var_os("SystemDrive").unwrap_or_else(|| "C:".into());
        locator.add_root(PathBuf::from(format!(
            "{}\\Riot Games\\League of Legends",
            drive.to_string_lossy()
        )));
        for variable in ["ProgramFiles", "ProgramFiles(x86)"] {
            if let Some(base) = env::var_os(variable) {
                let base = PathBuf::from(base);
                locator.add_root(base.join("Riot Games/League of Legends"));
                locator.add_root(base.join("League of Legends"));
            }
        }
        locator
    }
}
