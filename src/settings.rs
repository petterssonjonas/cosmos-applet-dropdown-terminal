// SPDX-License-Identifier: GPL-3.0-only
use cosmic::cosmic_config::{
    self, ConfigSet, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry,
};
use serde::{Deserialize, Serialize};

pub const CONFIG_ID: &str = "dev.jp.CosmosDropdownTerminal";
pub const CONFIG_VERSION: u64 = 1;

// Read only the dropdown keys. Terminal appearance and profiles belong to the backend.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, CosmicConfigEntry)]
pub struct Preferences {
    pub dropdown_height_percent: u8,
    pub dropdown_width_percent: u8,
    pub dropdown_always_on_top: bool,
    #[serde(default)]
    pub dropdown_close_on_focus_loss: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            dropdown_height_percent: 40,
            dropdown_width_percent: 100,
            dropdown_always_on_top: true,
            dropdown_close_on_focus_loss: false,
        }
    }
}

pub fn load() -> Result<Preferences, String> {
    let config =
        cosmic_config::Config::new(CONFIG_ID, CONFIG_VERSION).map_err(|error| error.to_string())?;
    load_from(&config)
}

fn load_from(config: &cosmic_config::Config) -> Result<Preferences, String> {
    match Preferences::get_entry(config) {
        Ok(preferences) => Ok(preferences),
        Err((errors, preferences)) => {
            let errors: Vec<_> = errors
                .into_iter()
                .filter(|error| !matches!(error, cosmic_config::Error::NotFound))
                .map(|error| error.to_string())
                .collect();
            if errors.is_empty() {
                Ok(preferences)
            } else {
                Err(errors.join("; "))
            }
        }
    }
}

pub fn save(original: &Preferences, draft: &Preferences) -> Result<(), String> {
    let config =
        cosmic_config::Config::new(CONFIG_ID, CONFIG_VERSION).map_err(|error| error.to_string())?;
    save_to(&config, original, draft)
}

fn save_to(
    config: &cosmic_config::Config,
    original: &Preferences,
    draft: &Preferences,
) -> Result<(), String> {
    // Read immediately before saving; queued watcher snapshots are not authoritative.
    let current = load_from(config)?;
    let transaction = config.transaction();
    macro_rules! stage {
        ($field:ident) => {
            if draft.$field != original.$field {
                if current.$field != original.$field && current.$field != draft.$field {
                    return Err(format!(
                        "{} changed outside this dialog. Cancel and reopen settings before saving.",
                        stringify!($field)
                    ));
                }
                transaction
                    .set(stringify!($field), draft.$field)
                    .map_err(|error| error.to_string())?;
            }
        };
    }
    stage!(dropdown_height_percent);
    stage!(dropdown_width_percent);
    stage!(dropdown_always_on_top);
    stage!(dropdown_close_on_focus_loss);
    // COSMIC commits keys individually, so callers must report possible partial writes.
    transaction.commit().map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn config() -> (std::path::PathBuf, cosmic_config::Config) {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let path = std::env::temp_dir().join(format!(
            "dropdown-settings-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        let config =
            cosmic_config::Config::with_custom_path(CONFIG_ID, CONFIG_VERSION, path.clone())
                .unwrap();
        (path, config)
    }

    #[test]
    fn save_preserves_external_changes_to_untouched_fields() {
        let (path, config) = config();
        config.set("dropdown_height_percent", 40_u8).unwrap();
        config.set("dropdown_width_percent", 100_u8).unwrap();
        config.set("dropdown_always_on_top", true).unwrap();
        let original = load_from(&config).unwrap();
        assert!(!original.dropdown_close_on_focus_loss);
        let mut draft = original.clone();
        draft.dropdown_height_percent = 75;
        draft.dropdown_close_on_focus_loss = true;
        config.set("dropdown_width_percent", 65_u8).unwrap();
        save_to(&config, &original, &draft).unwrap();
        draft.dropdown_width_percent = 65;
        assert_eq!(load_from(&config).unwrap(), draft);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn conflicting_external_change_fails_before_writing_any_key() {
        let (path, config) = config();
        let original = load_from(&config).unwrap();
        let mut draft = original.clone();
        draft.dropdown_height_percent = 75;
        draft.dropdown_width_percent = 50;
        draft.dropdown_close_on_focus_loss = true;
        config.set("dropdown_width_percent", 65_u8).unwrap();
        assert!(save_to(&config, &original, &draft).is_err());
        assert_eq!(
            load_from(&config).unwrap().dropdown_height_percent,
            original.dropdown_height_percent
        );
        assert!(!load_from(&config).unwrap().dropdown_close_on_focus_loss);
        std::fs::remove_dir_all(path).unwrap();
    }

    #[test]
    fn config_io_failure_reports_error_instead_of_success() {
        let (path, config) = config();
        let original = load_from(&config).unwrap();
        let mut draft = original.clone();
        draft.dropdown_height_percent = 75;
        // A directory cannot be atomically replaced by a config value.
        std::fs::create_dir_all(
            path.join("cosmic")
                .join(CONFIG_ID)
                .join("v1")
                .join("dropdown_height_percent"),
        )
        .unwrap();
        assert!(save_to(&config, &original, &draft).is_err());
        std::fs::remove_dir_all(path).unwrap();
    }
}
