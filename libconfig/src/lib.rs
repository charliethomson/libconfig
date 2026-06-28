mod error;
mod macros;

use figment::{
    Figment,
    providers::{Env, Format, Serialized, Toml},
};
use libpath::{config_path, config_path_no_create};
use serde::{Serialize, de::DeserializeOwned};
use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

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

/// Where a [`Loader`] reads the TOML config file from (if any).
#[derive(Debug, Clone)]
enum Source {
    /// Resolve the path from [`libpath`] using the module name. This is the
    /// historical desktop/tool behavior: the OS config dir (or libpath's base
    /// override, if set).
    Module(String),
    /// An explicit, caller-supplied file path — e.g. a deploy-controlled
    /// `/etc/<product>` bind mount. Never creates directories.
    Path(PathBuf),
    /// No file at all — defaults + env only. Never touches the filesystem.
    None,
}

/// Builder for loading configuration with full control over the file source, a
/// bare shared-env layer, the prefixed-env layer, and write-back.
///
/// Layers are merged in increasing precedence:
///
/// 1. the type's [`Default`]
/// 2. the TOML file (if the source has one and it exists)
/// 3. **bare shared env** — the unprefixed keys named via [`shared_env`](Loader::shared_env),
///    read with no prefix (e.g. fleet-wide `AUTH_ADMIN_KEY`, `OTLP_ENDPOINT`)
/// 4. **prefixed env** — `<PREFIX>_*` via [`env_prefix`](Loader::env_prefix)
///
/// The desktop/tool entry points ([`load`], [`store`], [`load_tracked`], and the
/// [`config!`](crate::config) macro) are thin wrappers over this builder and
/// behave exactly as before. Containerized services use [`Loader::path`] (a
/// deploy-controlled file) or [`Loader::pure_env`] (no file), combined with
/// `shared_env`, to read config without an OS-user-dir dependency or forced
/// `mkdir`.
///
/// ```ignore
/// // Containerized service: deploy file + fleet-shared env + own prefix.
/// let cfg = Loader::path("/etc/myproduct/config.toml")
///     .shared_env(["AUTH_ADMIN_KEY", "AUTH_TCP_ADDR", "OTLP_ENDPOINT", "PRODUCTION", "SAMPLE_RATE"])
///     .env_prefix("MYPRODUCT_")
///     .load::<ServiceConfig>()?;
/// ```
pub struct Loader<'a> {
    source: Source,
    env_prefix: Option<&'a str>,
    shared_env: Vec<&'a str>,
    write_back: bool,
}

impl<'a> Loader<'a> {
    /// Desktop/tool default: resolve `config_path(module)` via [`libpath`] and
    /// write the canonical config back to disk after loading. Honors libpath's
    /// base override and dir-creation policy.
    #[must_use]
    pub fn module(module: impl Into<String>) -> Self {
        Self {
            source: Source::Module(module.into()),
            env_prefix: None,
            shared_env: Vec::new(),
            write_back: true,
        }
    }

    /// Read from an explicit, caller-supplied path (e.g. a deploy-controlled
    /// bind mount). Never creates directories and, by default, never writes
    /// back — call [`write_back(true)`](Loader::write_back) to opt in.
    #[must_use]
    pub fn path(path: impl Into<PathBuf>) -> Self {
        Self {
            source: Source::Path(path.into()),
            env_prefix: None,
            shared_env: Vec::new(),
            write_back: false,
        }
    }

    /// No config file at all — pure defaults + env. Never touches the
    /// filesystem and never writes back.
    #[must_use]
    pub fn pure_env() -> Self {
        Self {
            source: Source::None,
            env_prefix: None,
            shared_env: Vec::new(),
            write_back: false,
        }
    }

    /// Set the prefix for the highest-precedence env layer (e.g. `"APP_"`
    /// enables `APP_SERVER_PORT`).
    #[must_use]
    pub fn env_prefix(mut self, prefix: &'a str) -> Self {
        self.env_prefix = Some(prefix);
        self
    }

    /// Merge a set of **unprefixed** env keys as a layer between the TOML file
    /// and the prefixed-env layer. Intended for fleet-shared variables such as
    /// `AUTH_ADMIN_KEY` or `OTLP_ENDPOINT` that are not namespaced per service.
    ///
    /// Keys are matched case-insensitively; each maps to the lower-cased config
    /// field of the same name (e.g. `OTLP_ENDPOINT` → `otlp_endpoint`).
    #[must_use]
    pub fn shared_env(mut self, keys: impl IntoIterator<Item = &'a str>) -> Self {
        self.shared_env = keys.into_iter().collect();
        self
    }

    /// Override whether the canonical config is written back to the source file
    /// after loading. Defaults to `true` for [`Loader::module`] and `false` for
    /// [`Loader::path`] / [`Loader::pure_env`]. Has no effect when there is no
    /// file (pure env).
    #[must_use]
    pub fn write_back(mut self, write_back: bool) -> Self {
        self.write_back = write_back;
        self
    }

    /// The file path this loader reads/writes, if any. `Module` sources avoid
    /// creating directories when not writing back.
    fn resolve_path(&self) -> Option<PathBuf> {
        match &self.source {
            Source::Module(module) if self.write_back => Some(config_path(module)),
            Source::Module(module) => Some(config_path_no_create(module)),
            Source::Path(path) => Some(path.clone()),
            Source::None => None,
        }
    }

    /// Build the layered figment for the given (optional) file path.
    fn figment<Config>(&self, path: Option<&Path>) -> Figment
    where
        Config: Serialize + DeserializeOwned + Default,
    {
        let mut figment = Figment::from(Serialized::defaults(Config::default()));

        if let Some(path) = path
            && path.exists()
        {
            figment = figment.merge(Toml::file(path));
        }

        if !self.shared_env.is_empty() {
            // figment lower-cases env keys; lower-case the filter keys to match
            // so callers can name the actual (upper-case) env vars.
            let lowered: Vec<String> = self.shared_env.iter().map(|k| k.to_lowercase()).collect();
            let keys: Vec<&str> = lowered.iter().map(String::as_str).collect();
            figment = figment.merge(Env::raw().only(&keys));
        }

        if let Some(prefix) = self.env_prefix {
            figment = figment.merge(Env::prefixed(prefix));
        }

        figment
    }

    /// Extract the config, self-healing a corrupt owned file when writing back.
    fn extract<Config>(&self, path: Option<&Path>) -> Result<Config, ConfigError>
    where
        Config: Serialize + DeserializeOwned + Default,
    {
        match self.figment::<Config>(path).extract::<Config>() {
            Ok(config) => Ok(config),
            // Only self-heal when we own a writable file that exists: the file
            // likely contains corrupted TOML (e.g. from a previous concurrent
            // write). Delete it and fall back to defaults + env so future runs
            // aren't permanently blocked. For read-only sources (a deploy path
            // or pure env) we never delete and simply surface the parse error.
            Err(_) if self.write_back && path.is_some_and(Path::exists) => {
                if let Some(path) = path {
                    let _ = std::fs::remove_file(path);
                }
                self.figment::<Config>(None)
                    .extract::<Config>()
                    .map_err(|e| ConfigError::Parse { inner_error: e.into() })
            }
            Err(e) => Err(ConfigError::Parse { inner_error: e.into() }),
        }
    }

    /// Load the configuration, writing the canonical form back to disk if
    /// write-back is enabled and the source has a file.
    pub fn load<Config>(&self) -> Result<Config, ConfigError>
    where
        Config: Serialize + DeserializeOwned + Default,
    {
        let path = self.resolve_path();
        let config = self.extract::<Config>(path.as_deref())?;

        if self.write_back
            && let Some(path) = &path
        {
            store_config(path, &config)?;
        }

        Ok(config)
    }

    /// Like [`load`](Loader::load), but returns a [`LoadedConfig`] that tracks
    /// the source file's mtime so [`LoadedConfig::store_checked`] can detect
    /// external modifications before writing.
    pub fn load_tracked<Config>(&self) -> Result<LoadedConfig<Config>, ConfigError>
    where
        Config: Serialize + DeserializeOwned + Default,
    {
        let path = self.resolve_path();
        let config = self.load::<Config>()?;
        // Capture mtime after load(), which may have written the canonical form.
        let mtime = path.as_deref().and_then(fs::get_mtime);
        Ok(LoadedConfig { config, path, mtime })
    }
}

pub fn load<Config: Serialize + DeserializeOwned + Default>(
    module: &str,
    env_prefix: Option<&str>,
) -> Result<Config, ConfigError> {
    let mut loader = Loader::module(module);
    if let Some(prefix) = env_prefix {
        loader = loader.env_prefix(prefix);
    }
    loader.load::<Config>()
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
    /// The backing file, if any. `None` for pure-env sources.
    path: Option<PathBuf>,
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
    ///
    /// For pure-env sources (no backing file) this is a no-op and returns `Ok(())`.
    pub fn store_checked(&self) -> Result<(), ConfigError> {
        match &self.path {
            Some(path) => fs::store_config_checked(path, &self.config, self.mtime),
            None => Ok(()),
        }
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
    let mut loader = Loader::module(module);
    if let Some(prefix) = env_prefix {
        loader = loader.env_prefix(prefix);
    }
    loader.load_tracked::<Config>()
}

pub trait ConfigExt: Serialize + DeserializeOwned + Default + Sized {
    fn module() -> &'static str;
    fn env_prefix() -> Option<&'static str>;
    /// Unprefixed, fleet-shared env keys merged between the TOML file and the
    /// prefixed-env layer. Defaults to none.
    #[must_use]
    fn shared_env() -> &'static [&'static str] {
        &[]
    }

    fn store(&self) -> Result<(), ConfigError> {
        crate::store(Self::module(), self)
    }
    fn load() -> Result<Self, ConfigError> {
        let mut loader = Loader::module(Self::module()).shared_env(Self::shared_env().iter().copied());
        if let Some(prefix) = Self::env_prefix() {
            loader = loader.env_prefix(prefix);
        }
        loader.load::<Self>()
    }
    fn load_tracked() -> Result<LoadedConfig<Self>, ConfigError> {
        let mut loader = Loader::module(Self::module()).shared_env(Self::shared_env().iter().copied());
        if let Some(prefix) = Self::env_prefix() {
            loader = loader.env_prefix(prefix);
        }
        loader.load_tracked::<Self>()
    }
}

#[cfg(test)]
mod tests {
    use libpath::config_path;
    use libproduct::product_name;
    use serde::{Deserialize, Serialize};

    use crate::{ConfigError, Loader, load, load_tracked, store};

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
    fn test_shared_env_layer() {
        // A pure-env load (no file, no OS-user-dir dependency) reading both an
        // unprefixed fleet-shared var and a prefixed service var, asserting the
        // shared layer sits below the prefixed layer.
        #[derive(Debug, Serialize, Deserialize, Default)]
        struct SharedCfg {
            shared_only_value: String,
            overlap_value: u32,
        }

        unsafe {
            std::env::set_var("SHARED_ONLY_VALUE", "from-shared");
            std::env::set_var("OVERLAP_VALUE", "1");
            std::env::set_var("SHTEST_OVERLAP_VALUE", "2");
        }

        let cfg = Loader::pure_env()
            .env_prefix("SHTEST_")
            .shared_env(["SHARED_ONLY_VALUE", "OVERLAP_VALUE"])
            .load::<SharedCfg>()
            .unwrap();
        assert_eq!(cfg.shared_only_value, "from-shared");
        assert_eq!(cfg.overlap_value, 2, "prefixed env must beat shared env");

        // Without declaring it shared, the bare var is ignored (stays default).
        let plain = Loader::pure_env().load::<SharedCfg>().unwrap();
        assert_eq!(plain.shared_only_value, "");

        unsafe {
            std::env::remove_var("SHARED_ONLY_VALUE");
            std::env::remove_var("OVERLAP_VALUE");
            std::env::remove_var("SHTEST_OVERLAP_VALUE");
        }
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
