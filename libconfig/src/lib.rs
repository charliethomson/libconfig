mod error;
mod macros;

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use libpath::config_path;
use serde::{Serialize, de::DeserializeOwned};

mod fs {
    use std::{io::Write, path::Path};

    use serde::{Serialize, de::DeserializeOwned};

    use crate::ConfigError;

    pub(super) fn store_config<Config>(path: &Path, config: &Config) -> Result<(), ConfigError>
    where
        Config: Serialize + DeserializeOwned + Default,
    {
        let buffer = toml::to_string(config).map_err(|e| ConfigError::Dump {
            inner_error: e.into(),
        })?;

        // Write to a per-process temp file then atomically rename into place.
        // This prevents concurrent writers from interleaving partial writes and
        // corrupting the TOML (rename(2) is atomic on POSIX).
        let tmp_path = path.with_extension(format!("tmp.{}", std::process::id()));

        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp_path)
            .map_err(|e| ConfigError::Open {
                path: tmp_path.display().to_string(),
                inner_error: e.into(),
            })?;

        file.write_all(buffer.as_bytes()).map_err(|e| ConfigError::Write {
            path: tmp_path.display().to_string(),
            inner_error: e.into(),
        })?;

        std::fs::rename(&tmp_path, path).map_err(|e| ConfigError::Write {
            path: path.display().to_string(),
            inner_error: e.into(),
        })?;

        Ok(())
    }
}

pub use crate::error::ConfigError;
use crate::fs::store_config;

pub fn load<Config: Serialize + DeserializeOwned + Default>(
    module: &str,
    env_prefix: Option<&str>,
) -> Result<Config, ConfigError> {
    let path = config_path(module);

    let mut figment = Figment::from(Serialized::defaults(Config::default()));

    if path.exists() {
        figment = figment.merge(Toml::file(&path));
    }

    if let Some(prefix) = env_prefix {
        figment = figment.merge(Env::prefixed(prefix));
    }

    let config = match figment.extract::<Config>() {
        Ok(c) => c,
        Err(_) if path.exists() => {
            // The file likely contains corrupted TOML (e.g. from a previous
            // concurrent write). Delete it and fall back to defaults + env so
            // future runs aren't permanently blocked.
            let _ = std::fs::remove_file(&path);
            let mut fallback = Figment::from(Serialized::defaults(Config::default()));
            if let Some(prefix) = env_prefix {
                fallback = fallback.merge(Env::prefixed(prefix));
            }
            fallback.extract::<Config>().map_err(|e| ConfigError::Parse {
                inner_error: e.into(),
            })?
        }
        Err(e) => return Err(ConfigError::Parse { inner_error: e.into() }),
    };

    store_config(&path, &config)?;

    Ok(config)
}

pub fn store<Config: Serialize + DeserializeOwned + Default>(
    module: &str,
    config: &Config,
) -> Result<(), ConfigError> {
    let path = config_path(module);
    store_config(&path, config)?;

    Ok(())
}

pub trait ConfigExt: Serialize + DeserializeOwned + Default + Sized {
    fn module() -> &'static str;
    fn env_prefix() -> Option<&'static str>;

    fn store(&self) -> Result<(), ConfigError> {
        crate::store(Self::module(), self)
    }
    fn load() -> Result<Self, ConfigError> {
        crate::load(Self::module(), Self::env_prefix())
    }
}

#[cfg(test)]
mod tests {
    use libpath::config_path;
    use libproduct::product_name;
    use serde::{Deserialize, Serialize};

    use crate::{load, store};

    product_name!("dev.thmsn.unit_tests");

    #[derive(Debug, Clone, Serialize, Deserialize)]
    struct TestConfig {
        pub name: String,
    }
    impl Default for TestConfig {
        fn default() -> Self {
            Self {
                name: "Omg im testing!".into(),
            }
        }
    }

    #[test]
    fn test_first_load() {
        let module = "test_first_load";
        let _ = PRODUCT_NAME.set_global();
        let path = config_path(module);
        if path.exists() {
            std::fs::remove_file(&path).expect("Failed to remove existing config");
        }

        let config = load::<TestConfig>(module, None);
        assert!(config.is_ok());
        let config = config.unwrap();

        assert!(config.name == TestConfig::default().name);
        assert!(
            path.exists(),
            "Default config file should have been created"
        );
    }

    #[test]
    fn test_modified_load() {
        let module = "test_modified_load";
        let _ = PRODUCT_NAME.set_global();
        let updated_name = "omg im something different now!";

        let path = config_path(module);
        if path.exists() {
            std::fs::remove_file(&path).expect("Failed to remove existing config");
        }

        let config = load::<TestConfig>(module, None);
        assert!(config.is_ok());
        let mut config = config.unwrap();

        assert!(config.name == TestConfig::default().name);
        assert!(
            path.exists(),
            "Default config file should have been created"
        );

        config.name = updated_name.to_string();
        store(module, &config).expect("Store failed");

        let config = load::<TestConfig>(module, None);
        assert!(config.is_ok());
        let config = config.unwrap();

        assert!(config.name == updated_name);
    }

    #[test]
    fn test_env_override() {
        let module = "test_env_override";
        let _ = PRODUCT_NAME.set_global();
        let path = config_path(module);
        if path.exists() {
            std::fs::remove_file(&path).expect("Failed to remove existing config");
        }

        let env_name = "omg im something from the environment now!";

        unsafe {
            std::env::set_var("LIBCONFIG_NAME", env_name);
        }

        let config = load::<TestConfig>(module, Some("LIBCONFIG_"));
        assert!(config.is_ok());
        let config = config.unwrap();

        assert!(config.name == env_name);
        assert!(path.exists(), "Config file should have been created");
    }

    #[test]
    fn test_env_override_stores() {
        let module = "test_env_override_stores";
        let _ = PRODUCT_NAME.set_global();
        let path = config_path(module);
        if path.exists() {
            std::fs::remove_file(&path).expect("Failed to remove existing config");
        }

        let env_name = "omg im something from the environment now!";

        unsafe {
            std::env::set_var("LIBCONFIG_NAME", env_name);
        }

        let config = load::<TestConfig>(module, Some("LIBCONFIG_"));
        assert!(config.is_ok());
        let config = config.unwrap();

        assert!(config.name == env_name);
        assert!(path.exists(), "Config file should have been created");

        unsafe {
            std::env::remove_var("LIBCONFIG_NAME");
        }

        let config = load::<TestConfig>(module, None);
        assert!(config.is_ok());
        let config = config.unwrap();
        assert!(config.name == env_name);
    }
}
