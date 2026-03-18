mod error;
mod macros;

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use libpath::config_path;
use serde::{Serialize, de::DeserializeOwned};
use std::{path::PathBuf, time::SystemTime};

mod fs {
    use std::{
        io::Write,
        path::Path,
        sync::atomic::{AtomicU64, Ordering},
        time::SystemTime,
    };

    use serde::{Serialize, de::DeserializeOwned};

    use crate::ConfigError;

    static STORE_COUNTER: AtomicU64 = AtomicU64::new(0);

    pub(super) fn get_mtime(path: &Path) -> Option<SystemTime> {
        std::fs::metadata(path).ok()?.modified().ok()
    }

    pub(super) fn store_config_checked<Config>(
        path: &Path,
        config: &Config,
        expected_mtime: Option<SystemTime>,
    ) -> Result<(), ConfigError>
    where
        Config: Serialize + DeserializeOwned + Default,
    {
        if get_mtime(path) != expected_mtime {
            return Err(ConfigError::Stale);
        }
        store_config(path, config)
    }

    pub(super) fn store_config<Config>(path: &Path, config: &Config) -> Result<(), ConfigError>
    where
        Config: Serialize + DeserializeOwned + Default,
    {
        let buffer = toml::to_string(config).map_err(|e| ConfigError::Dump {
            inner_error: e.into(),
        })?;

        // Write to a unique temp file then atomically rename into place.
        // Using pid + per-call counter ensures uniqueness across both processes
        // and threads within the same process. rename(2) is atomic on POSIX.
        let seq = STORE_COUNTER.fetch_add(1, Ordering::Relaxed);
        let tmp_path = path.with_extension(format!("tmp.{}.{}", std::process::id(), seq));

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

/// A loaded configuration value bundled with the file's modification time at load.
///
/// Use [`store_checked`](LoadedConfig::store_checked) to write back the config only if the file
/// has not been modified externally since it was loaded. Returns [`ConfigError::Stale`] if the
/// file's mtime has changed, allowing callers to detect and handle concurrent edits.
pub struct LoadedConfig<Config> {
    config: Config,
    path: PathBuf,
    mtime: Option<SystemTime>,
}

impl<Config> LoadedConfig<Config> {
    pub fn mtime(&self) -> Option<SystemTime> {
        self.mtime
    }

    pub fn into_inner(self) -> Config {
        self.config
    }
}

impl<Config: Serialize + DeserializeOwned + Default> LoadedConfig<Config> {
    /// Write the config back to disk, returning [`ConfigError::Stale`] if the file was modified
    /// externally since it was loaded.
    pub fn store_checked(&self) -> Result<(), ConfigError> {
        fs::store_config_checked(&self.path, &self.config, self.mtime)
    }
}

impl<Config> std::ops::Deref for LoadedConfig<Config> {
    type Target = Config;
    fn deref(&self) -> &Config {
        &self.config
    }
}

impl<Config> std::ops::DerefMut for LoadedConfig<Config> {
    fn deref_mut(&mut self) -> &mut Config {
        &mut self.config
    }
}

/// Like [`load`], but returns a [`LoadedConfig`] that tracks the file's mtime so that
/// [`LoadedConfig::store_checked`] can detect external modifications before writing.
pub fn load_tracked<Config: Serialize + DeserializeOwned + Default>(
    module: &str,
    env_prefix: Option<&str>,
) -> Result<LoadedConfig<Config>, ConfigError> {
    let path = config_path(module);
    let config = load::<Config>(module, env_prefix)?;
    // Capture mtime after load(), which itself writes the canonical form to disk.
    let mtime = fs::get_mtime(&path);
    Ok(LoadedConfig { config, path, mtime })
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
    fn load_tracked() -> Result<LoadedConfig<Self>, ConfigError> {
        crate::load_tracked(Self::module(), Self::env_prefix())
    }
}

#[cfg(test)]
mod tests {
    use libpath::config_path;
    use libproduct::product_name;
    use serde::{Deserialize, Serialize};

    use crate::{ConfigError, load, load_tracked, store};

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
    fn test_stale_detection() {
        let module = "test_stale_detection";
        let _ = PRODUCT_NAME.set_global();
        let path = config_path(module);
        if path.exists() {
            std::fs::remove_file(&path).expect("Failed to remove existing config");
        }

        let loaded = load_tracked::<TestConfig>(module, None).unwrap();

        // Simulate an external edit. Sleep briefly so the mtime advances on
        // filesystems with sub-second resolution.
        std::thread::sleep(std::time::Duration::from_millis(10));
        std::fs::write(&path, "name = \"externally modified\"\n")
            .expect("Failed to simulate external edit");

        let result = loaded.store_checked();
        assert!(
            matches!(result, Err(ConfigError::Stale)),
            "expected Stale error, got: {result:?}"
        );
    }

    #[test]
    fn test_store_checked_succeeds_when_unmodified() {
        let module = "test_store_checked_unmodified";
        let _ = PRODUCT_NAME.set_global();
        let path = config_path(module);
        if path.exists() {
            std::fs::remove_file(&path).expect("Failed to remove existing config");
        }

        let loaded = load_tracked::<TestConfig>(module, None).unwrap();
        let result = loaded.store_checked();
        assert!(result.is_ok(), "expected Ok, got: {result:?}");
        assert!(path.exists());
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
