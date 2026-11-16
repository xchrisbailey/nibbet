use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use crate::{Space, agents};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EditorMode {
    /// The Notion-style block view (ADR 0003).
    #[default]
    Block,
    /// Plain Markdown source.
    Raw,
}

const SUPPORT: &str = "Library/Application Support";

/// What [`Settings::migrate_from_sett`] did.
#[derive(Debug)]
pub enum SettMigration {
    /// No sett install here, or it was carried over before.
    NotNeeded,
    /// The settings came over; the data stays where it was.
    SettingsOnly,
    /// The settings came over and `~/sett` moved to `~/nibbet`.
    Moved,
    /// The settings came over, but `~/sett` couldn't be moved, so the app
    /// keeps using it there.
    MoveFailed(io::Error),
}

/// App settings stored in `settings.toml`, outside the data root.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Settings {
    pub data_root: PathBuf,
    pub editor_mode: EditorMode,
    pub last_space: Space,
}

impl Settings {
    /// `~/Library/Application Support/nibbet/settings.toml`, or `None` when
    /// `$HOME` isn't set.
    pub fn default_path() -> Option<PathBuf> {
        Some(Settings::path_in(&home_dir()?))
    }

    /// Where the settings file lives for this home folder.
    pub fn path_in(home: &Path) -> PathBuf {
        home.join(SUPPORT).join("nibbet/settings.toml")
    }

    /// Carries an install from when the app was called sett over to nibbet,
    /// once: the settings file is copied (the old one stays), and the data
    /// root moves from `~/sett` to `~/nibbet`. Data moves only when it sits
    /// at sett's default place, is sett's own (its `AGENTS.md` says so) and
    /// `~/nibbet` is free; anything else keeps its data root, so nothing a
    /// user pointed elsewhere, or that isn't sett's, is touched.
    pub fn migrate_from_sett(home: &Path) -> io::Result<SettMigration> {
        let path = Settings::path_in(home);
        let old_path = home.join(SUPPORT).join("sett/settings.toml");
        let old_root = home.join("sett");
        let new_root = home.join("nibbet");
        let is_sett_data = agents::written_by_sett(&old_root);
        if path.exists() || !(old_path.is_file() || is_sett_data) {
            return Ok(SettMigration::NotNeeded);
        }
        // sett's own default, for a file without `data_root`.
        let sett_defaults = Settings {
            data_root: old_root.clone(),
            ..Settings::defaults(home)
        };
        let mut settings = Settings::load_onto(&old_path, sett_defaults);
        let mut outcome = SettMigration::SettingsOnly;
        if settings.data_root == old_root {
            if !old_root.exists() {
                settings.data_root = new_root;
            } else if is_sett_data && !new_root.exists() {
                match fs::rename(&old_root, &new_root) {
                    Ok(()) => {
                        settings.data_root = new_root;
                        outcome = SettMigration::Moved;
                    }
                    Err(err) => outcome = SettMigration::MoveFailed(err),
                }
            }
        }
        settings.save(&path)?;
        Ok(outcome)
    }

    pub fn defaults(home: &Path) -> Settings {
        Settings {
            data_root: home.join("nibbet"),
            editor_mode: EditorMode::default(),
            last_space: Space::Personal,
        }
    }

    /// Never fails: a missing or unreadable file gives defaults, and each
    /// field that is missing or invalid falls back on its own so one bad
    /// hand edit doesn't reset the rest.
    pub fn load(path: &Path, home: &Path) -> Settings {
        Settings::load_onto(path, Settings::defaults(home))
    }

    fn load_onto(path: &Path, defaults: Settings) -> Settings {
        let mut settings = defaults;
        let Some(table) = fs::read_to_string(path)
            .ok()
            .and_then(|t| t.parse::<toml::Table>().ok())
        else {
            return settings;
        };
        if let Some(v) = field(&table, "data_root") {
            settings.data_root = v;
        }
        if let Some(v) = field(&table, "editor_mode") {
            settings.editor_mode = v;
        }
        if let Some(v) = field(&table, "last_space") {
            settings.last_space = v;
        }
        settings
    }

    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let text = toml::to_string(self).map_err(io::Error::other)?;
        fs::write(path, text)
    }
}

pub(crate) fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME")
        .filter(|h| !h.is_empty())
        .map(PathBuf::from)
}

fn field<T: DeserializeOwned>(table: &toml::Table, key: &str) -> Option<T> {
    table.get(key)?.clone().try_into().ok()
}
