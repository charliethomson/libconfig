mod error;

use libpath::config_path;
use serde::{Serialize, de::DeserializeOwned};

pub use crate::error::ConfigError;

mod fs {
    use std::{io::Write, path::Path};

    use serde::{Serialize, de::DeserializeOwned};

    use crate::ConfigError;

    pub(super) fn load_config<Config>(path: &Path) -> Result<Option<Config>, ConfigError>
    where
        Config: Serialize + DeserializeOwned + Default,
    {
        if !path.exists() {
            return Ok(None);
        }

        // TODO: should this fail or just Ok(None)
        let content = std::fs::read(path).map_err(|e| ConfigError::Read {
            path: path.to_path_buf().display().to_string(),
            inner_error: e.into(),
        })?;

        toml::from_slice(&content).map_err(|e| ConfigError::Parse {
            path: path.to_path_buf().display().to_string(),
            content: content.clone(),
            inner_error: e.into(),
        })
    }

    pub(super) fn store_config<Config>(path: &Path, config: &Config) -> Result<(), ConfigError>
    where
        Config: Serialize + DeserializeOwned + Default,
    {
        let buffer = toml::to_string(config).map_err(|e| ConfigError::Dump {
            inner_error: e.into(),
        })?;

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(path)
            .map_err(|e| ConfigError::Open {
                path: path.to_path_buf().display().to_string(),
                inner_error: e.into(),
            })?;

        file.write(buffer.as_bytes())
            .map_err(|e| ConfigError::Write {
                path: path.to_path_buf().display().to_string(),
                inner_error: e.into(),
            })?;
        Ok(())
    }
}

pub fn merge_config<Config, Mutator>(module: &str, mutator: Mutator) -> Result<Config, ConfigError>
where
    Config: Serialize + DeserializeOwned + Default,
    Mutator: FnOnce(&mut Config),
{
    let path = config_path(module);

    let mut base = fs::load_config(&path)?.unwrap_or_default();
    mutator(&mut base);

    fs::store_config(&path, &base)?;

    Ok(base)
}

pub fn load_config<Config>(module: &str) -> Result<Option<Config>, ConfigError>
where
    Config: Serialize + DeserializeOwned + Default,
{
    let path = config_path(module);

    fs::load_config(&path)
}

pub fn store_config<Config>(module: &str, config: &Config) -> Result<(), ConfigError>
where
    Config: Serialize + DeserializeOwned + Default,
{
    let path = config_path(module);

    fs::store_config(&path, config)
}
