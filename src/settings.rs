// SPDX-License-Identifier: GPL-3.0-only
use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};

pub const CONFIG_ID: &str = "dev.jp.CosmosDropdownTerminal";
pub const CONFIG_VERSION: u64 = 1;

// Read only the dropdown keys. Terminal appearance and profiles belong to the backend.
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq, CosmicConfigEntry)]
pub struct Preferences {
    pub dropdown_height_percent: u8,
    pub dropdown_width_percent: u8,
    pub dropdown_always_on_top: bool,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            dropdown_height_percent: 40,
            dropdown_width_percent: 100,
            dropdown_always_on_top: true,
        }
    }
}

pub fn load() -> Result<Preferences, String> {
    let config =
        cosmic_config::Config::new(CONFIG_ID, CONFIG_VERSION).map_err(|error| error.to_string())?;
    match Preferences::get_entry(&config) {
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
