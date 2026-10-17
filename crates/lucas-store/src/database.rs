use crate::{
    Calibration, GameSummary, ProbePoint, Result, Settings, StoreError, StoredEvent,
    StoredEventKind, atomic_write, default_data_dir,
};
use lucas_core::DeathClassification;
use rusqlite::{Connection, OptionalExtension, params};
use std::{
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

pub const DATABASE_NAME: &str = "games.sqlite3";
const DATABASE_VERSION: u32 = 1;

/// Owns the database handle. One owner per application; do not open other handles
/// while deleting data. No network, camera, trace or feature-writing methods.
pub struct Store {
    directory: PathBuf,
    connection: Connection,
}

impl Store {
    pub fn open() -> Result<Self> {
        Self::open_in(default_data_dir()?)
    }

    /// Explicit override for tests/developer sessions. This must be a dedicated
    /// LUCAS directory, not a general documents or downloads directory.
    pub fn open_in(directory: impl AsRef<Path>) -> Result<Self> {
        fs::create_dir_all(directory.as_ref())?;
        let directory = fs::canonicalize(directory)?;
        let mut connection = Connection::open(directory.join(DATABASE_NAME))?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.pragma_update(None, "foreign_keys", "ON")?;
        migrate(&mut connection)?;
        Ok(Self {
            directory,
            connection,
        })
    }

    pub fn data_folder(&self) -> &Path {
        &self.directory
    }

    pub fn load_settings(&self) -> Result<Settings> {
        let path = self.directory.join("settings.toml");
        let text = match fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Settings::default());
            }
            Err(error) => return Err(error.into()),
        };
        let settings: Settings = toml::from_str(&text)?;
        settings.validate()?;
        Ok(settings)
    }

    pub fn save_settings(&self, settings: &Settings) -> Result<()> {
        settings.validate()?;
        atomic_write(
            &self.directory.join("settings.toml"),
            toml::to_string_pretty(settings)?.as_bytes(),
        )
    }

    pub fn load_calibration(&self) -> Result<Option<Calibration>> {
        let bytes = match fs::read(self.directory.join("calibration.json")) {
            Ok(bytes) => bytes,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(error) => return Err(error.into()),
        };
        let calibration: Calibration = serde_json::from_slice(&bytes)?;
        calibration.validate()?;
        Ok(Some(calibration))
    }

    pub fn save_calibration(&self, calibration: &Calibration) -> Result<()> {
        calibration.validate()?;
        atomic_write(
            &self.directory.join("calibration.json"),
            &serde_json::to_vec(calibration)?,
        )
    }

    /// Atomic, idempotent upsert by game ID. Retry replaces the entire event set
    /// rather than duplicating deaths. Invalid input leaves the old game intact.
    pub fn save_summary(&mut self, summary: &GameSummary) -> Result<()> {
        summary.validate()?;
        let mut snapshot = summary.clone();
        snapshot.events.clear();
        let payload = serde_json::to_string(&snapshot)?;
        let transaction = self.connection.transaction()?;
        transaction.execute(
            "INSERT INTO games (id, ended_at_unix_ms, probe_game, compliance, standard_ms,
                standard_changed, valid_fraction, snapshot_json)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(id) DO UPDATE SET ended_at_unix_ms=excluded.ended_at_unix_ms,
                probe_game=excluded.probe_game, compliance=excluded.compliance,
                standard_ms=excluded.standard_ms, standard_changed=excluded.standard_changed,
                valid_fraction=excluded.valid_fraction, snapshot_json=excluded.snapshot_json",
            params![
                summary.id,
                sql_integer(summary.context.ended_at_unix_ms)?,
                summary.context.probe_game,
                summary.metrics.compliance,
                sql_integer(summary.initial_level.standard_ms)?,
                summary.standard_changed,
                summary.context.tracking.valid_fraction(),
                payload
            ],
        )?;
        transaction.execute("DELETE FROM events WHERE game_id=?1", [&summary.id])?;
        {
            let mut insert = transaction.prepare(
                "INSERT INTO events (game_id, ordinal, game_time_ms, kind, classification,
                    seconds_blind, tracked_fraction) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            )?;
            for (ordinal, event) in summary.events.iter().enumerate() {
                let (kind, classification, seconds, tracked) = match event.kind {
                    StoredEventKind::Check => ("check", None, None, None),
                    StoredEventKind::Pulse => ("pulse", None, None, None),
                    StoredEventKind::Death {
                        classification,
                        seconds_blind,
                        tracked_fraction,
                    } => (
                        "death",
                        Some(class_name(classification)),
                        seconds_blind,
                        Some(tracked_fraction),
                    ),
                };
                insert.execute(params![
                    summary.id,
                    ordinal as i64,
                    sql_integer(event.game_time_ms)?,
                    kind,
                    classification,
                    seconds,
                    tracked
                ])?;
            }
        }
        transaction.commit()?;
        Ok(())
    }

    pub fn summary(&self, game_id: &str) -> Result<Option<GameSummary>> {
        let payload: Option<String> = self
            .connection
            .query_row(
                "SELECT snapshot_json FROM games WHERE id=?1",
                [game_id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(payload) = payload else {
            return Ok(None);
        };
        let mut summary: GameSummary = serde_json::from_str(&payload)?;
        summary.events = self.events(game_id)?;
        summary.validate()?;
        Ok(Some(summary))
    }

    pub fn latest_summary(&self) -> Result<Option<GameSummary>> {
        let id: Option<String> = self
            .connection
            .query_row(
                "SELECT id FROM games ORDER BY ended_at_unix_ms DESC, id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()?;
        id.map(|id| self.summary(&id))
            .transpose()
            .map(Option::flatten)
    }

    /// Stable event-time order, including delayed deaths and equal timestamps.
    pub fn events(&self, game_id: &str) -> Result<Vec<StoredEvent>> {
        let mut statement = self.connection.prepare(
            "SELECT game_time_ms, kind, classification, seconds_blind, tracked_fraction
             FROM events WHERE game_id=?1 ORDER BY game_time_ms, ordinal",
        )?;
        let rows = statement.query_map([game_id], |row| {
            Ok((
                read_unsigned(row, 0)?,
                row.get::<_, String>(1)?,
                row.get::<_, Option<String>>(2)?,
                row.get::<_, Option<f64>>(3)?,
                row.get::<_, Option<f64>>(4)?,
            ))
        })?;
        rows.map(|row| {
            let (game_time_ms, kind, class, seconds_blind, tracked) = row?;
            let kind = match kind.as_str() {
                "check" => StoredEventKind::Check,
                "pulse" => StoredEventKind::Pulse,
                "death" => StoredEventKind::Death {
                    classification: match class.as_deref() {
                        Some("blind") => DeathClassification::Blind,
                        Some("seen") => DeathClassification::Seen,
                        Some("unknown") => DeathClassification::Unknown,
                        _ => return Err(StoreError::Invalid("invalid death classification")),
                    },
                    seconds_blind,
                    tracked_fraction: tracked
                        .ok_or(StoreError::Invalid("missing death coverage"))?,
                },
                _ => return Err(StoreError::Invalid("invalid event kind")),
            };
            Ok(StoredEvent { game_time_ms, kind })
        })
        .collect()
    }

    /// Oldest to newest. Undefined C remains None, never a fabricated zero.
    pub fn probe_trend(&self) -> Result<Vec<ProbePoint>> {
        let mut statement = self.connection.prepare(
            "SELECT id, ended_at_unix_ms, compliance, standard_ms, standard_changed, valid_fraction
             FROM games WHERE probe_game=1 ORDER BY ended_at_unix_ms, id",
        )?;
        Ok(statement
            .query_map([], |row| {
                Ok(ProbePoint {
                    game_id: row.get(0)?,
                    ended_at_unix_ms: read_unsigned(row, 1)?,
                    compliance: row.get(2)?,
                    standard_ms: read_unsigned(row, 3)?,
                    standard_changed: row.get(4)?,
                    valid_fraction: row.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?)
    }

    /// Consumes the owner and closes SQLite BEFORE removing files (Windows-safe).
    /// Caller must stop capture/trace/report writers and release other store
    /// handles first. Does not follow directory symlinks when removing traces.
    /// Preserves unrelated files in an overridden directory. Errors are surfaced;
    /// callers may reopen the same directory to retry after releasing other handles.
    pub fn delete_all_data(self) -> Result<()> {
        let Self {
            directory,
            connection,
        } = self;
        connection.close().map_err(|(_, error)| error)?;
        let mut paths: Vec<PathBuf> = [
            DATABASE_NAME,
            "games.sqlite3-wal",
            "games.sqlite3-shm",
            "games.sqlite3-journal",
            "settings.toml",
            "calibration.json",
            "traces",
            "reports",
        ]
        .into_iter()
        .map(|name| directory.join(name))
        .collect();
        // Interrupted atomic writes can survive a process crash. They use a
        // private prefix so deletion need not sweep unrelated temporary files.
        for entry in fs::read_dir(&directory)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".lucas-write-")
                && !entry.file_type()?.is_dir()
            {
                paths.push(entry.path());
            }
        }
        for path in paths {
            let metadata = match fs::symlink_metadata(&path) {
                Ok(metadata) => metadata,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
                Err(error) => return Err(error.into()),
            };
            if metadata.is_dir() && !metadata.file_type().is_symlink() {
                fs::remove_dir_all(path)?;
            } else {
                #[cfg(windows)]
                if metadata.file_type().is_symlink() {
                    use std::os::windows::fs::FileTypeExt;
                    if metadata.file_type().is_symlink_dir() {
                        fs::remove_dir(path)?;
                        continue;
                    }
                }
                fs::remove_file(path)?;
            }
        }
        Ok(())
    }
}

fn sql_integer(value: u64) -> Result<i64> {
    i64::try_from(value).map_err(|_| StoreError::Invalid("timestamp exceeds SQLite integer range"))
}

fn read_unsigned(row: &rusqlite::Row<'_>, index: usize) -> rusqlite::Result<u64> {
    let value: i64 = row.get(index)?;
    u64::try_from(value).map_err(|_| rusqlite::Error::IntegralValueOutOfRange(index, value))
}

fn class_name(class: DeathClassification) -> &'static str {
    match class {
        DeathClassification::Blind => "blind",
        DeathClassification::Seen => "seen",
        DeathClassification::Unknown => "unknown",
    }
}

fn migrate(connection: &mut Connection) -> Result<()> {
    // IMMEDIATE prevents two startup owners both observing an empty schema.
    let transaction =
        connection.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
    let version: u32 = transaction.pragma_query_value(None, "user_version", |row| row.get(0))?;
    if version > DATABASE_VERSION {
        return Err(StoreError::Schema {
            kind: "database",
            version,
        });
    }
    if version == 0 {
        transaction.execute_batch(
            "CREATE TABLE games (
                id TEXT PRIMARY KEY NOT NULL,
                ended_at_unix_ms INTEGER NOT NULL CHECK (ended_at_unix_ms >= 0),
                probe_game INTEGER NOT NULL CHECK (probe_game IN (0,1)),
                compliance REAL CHECK (compliance BETWEEN 0 AND 1),
                standard_ms INTEGER NOT NULL CHECK (standard_ms > 0),
                standard_changed INTEGER NOT NULL CHECK (standard_changed IN (0,1)),
                valid_fraction REAL CHECK (valid_fraction BETWEEN 0 AND 1),
                snapshot_json TEXT NOT NULL
             );
             CREATE INDEX games_recent ON games(ended_at_unix_ms, id);
             CREATE INDEX games_probes ON games(probe_game, ended_at_unix_ms, id);
             CREATE TABLE events (
                game_id TEXT NOT NULL REFERENCES games(id) ON DELETE CASCADE,
                ordinal INTEGER NOT NULL,
                game_time_ms INTEGER NOT NULL CHECK (game_time_ms >= 0),
                kind TEXT NOT NULL CHECK (kind IN ('check','pulse','death')),
                classification TEXT CHECK (classification IN ('blind','seen','unknown')),
                seconds_blind REAL CHECK (seconds_blind >= 0),
                tracked_fraction REAL CHECK (tracked_fraction BETWEEN 0 AND 1),
                PRIMARY KEY(game_id, ordinal),
                CHECK ((kind='death' AND classification IS NOT NULL AND tracked_fraction IS NOT NULL)
                    OR (kind!='death' AND classification IS NULL AND seconds_blind IS NULL
                        AND tracked_fraction IS NULL))
             );
             CREATE INDEX events_time ON events(game_id, game_time_ms, ordinal);
             PRAGMA user_version=1;"
        )?;
    }
    transaction.commit()?;
    Ok(())
}
