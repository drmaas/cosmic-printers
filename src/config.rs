// SPDX-License-Identifier: GPL-3.0-only

use cosmic::cosmic_config::{self, CosmicConfigEntry, cosmic_config_derive::CosmicConfigEntry};
use serde::{Deserialize, Serialize};

/// When the panel button is drawn.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ShowIcon {
    Always,
    #[default]
    Printers,
    Jobs,
}

impl ShowIcon {
    pub const fn index(self) -> usize {
        match self {
            Self::Always => 0,
            Self::Printers => 1,
            Self::Jobs => 2,
        }
    }

    pub const fn from_index(index: usize) -> Self {
        match index {
            0 => Self::Always,
            1 => Self::Printers,
            _ => Self::Jobs,
        }
    }
}

#[derive(Debug, Default, Clone, CosmicConfigEntry, Eq, PartialEq)]
#[version = 1]
pub struct Config {
    pub show_icon: ShowIcon,
}

pub fn load() -> Config {
    cosmic_config::Config::new(crate::app::APP_ID, Config::VERSION)
        .map(|context| match Config::get_entry(&context) {
            Ok(config) => config,
            Err((_, config)) => config,
        })
        .unwrap_or_default()
}

pub fn save(config: &Config) {
    let Ok(context) = cosmic_config::Config::new(crate::app::APP_ID, Config::VERSION) else {
        eprintln!("cosmic-printers: could not open config");
        return;
    };
    if let Err(error) = config.write_entry(&context) {
        eprintln!("cosmic-printers: could not save config: {error}");
    }
}

#[cfg(test)]
mod tests {
    use super::ShowIcon;

    #[test]
    fn show_icon_index_round_trips() {
        for icon in [ShowIcon::Always, ShowIcon::Printers, ShowIcon::Jobs] {
            assert_eq!(ShowIcon::from_index(icon.index()), icon);
        }
    }
}
