use std::fs;

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

use crate::{i18n::Locale, paths::Paths};

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Config {
    #[serde(default)]
    pub locale: Locale,
}

impl Config {
    pub fn load(paths: &Paths) -> Result<Self> {
        if !paths.config.exists() {
            return Ok(Self::default());
        }
        serde_json::from_slice(
            &fs::read(&paths.config)
                .with_context(|| format!("failed to read {}", paths.config.display()))?,
        )
        .with_context(|| format!("failed to parse {}", paths.config.display()))
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        fs::create_dir_all(&paths.home)?;
        let mut contents = serde_json::to_vec_pretty(self)?;
        contents.push(b'\n');
        fs::write(&paths.config, contents)
            .with_context(|| format!("failed to write {}", paths.config.display()))
    }
}
