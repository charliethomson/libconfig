# libconfig

A Rust library for loading, storing, and managing application configuration backed by TOML files with optional environment variable overrides.

## Features

- Loads config from a TOML file resolved via [`libpath`](https://github.com/charliethomson/libpath)
- Merges values in priority order: type defaults → TOML file → environment variables
- Writes atomically (temp file + `rename(2)`) to avoid partial writes
- Recovers from corrupt config files by falling back to defaults
- Detects external edits via mtime tracking (`load_tracked` / `store_checked`)
- `config!` macro for ergonomic lazy-loaded static configs
- `ConfigExt` trait for `load()` / `store()` / `load_tracked()` methods on your config type

## Usage

### `config!` macro

```rust
use libconfig::{ConfigExt, config};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
struct AppConfig {
    port: u16,
    debug: bool,
}

// Basic static config
config! {
    pub static APP_CONFIG: AppConfig = {
        module: "myapp",
    }
}

// With env var overrides (e.g. APP_PORT=9000)
config! {
    pub static APP_CONFIG: AppConfig = {
        module: "myapp",
        env_prefix: "APP_",
    }
}

// With ConfigExt trait (adds .load() and .store() to AppConfig)
config! {
    pub static APP_CONFIG: AppConfig = {
        module: "myapp",
        env_prefix: "APP_",
        impl_trait,
    }
}
```

The static is a `LazyLock<T>` — config is loaded on first access and panics if the file exists but cannot be parsed.

### Functional API

```rust
use libconfig::{load, store};

// Load config (creates the file with defaults if it doesn't exist)
let config = load::<AppConfig>("myapp", Some("APP_"))?;

// Persist changes
store("myapp", &config)?;
```

### `ConfigExt` trait

Implement the trait manually or use `impl_trait` in the `config!` macro:

```rust
impl ConfigExt for AppConfig {
    fn module() -> &'static str { "myapp" }
    fn env_prefix() -> Option<&'static str> { Some("APP_") }
}

let mut config = AppConfig::load()?;
config.port = 9000;
config.store()?;
```

### Detecting external edits with `load_tracked`

Use `load_tracked` (or `ConfigExt::load_tracked`) when you hold a config in memory and need to
guard against concurrent edits to the file before writing it back. The returned `LoadedConfig<T>`
records the file's mtime at load time; `store_checked` compares that mtime before writing and
returns `ConfigError::Stale` if the file was modified in the meantime.

```rust
use libconfig::{ConfigExt, ConfigError};

// Load and record the file's mtime
let mut loaded = AppConfig::load_tracked()?;

// ... time passes, user may have edited the file on disk ...

loaded.port = 9000;

match loaded.store_checked() {
    Ok(()) => { /* written successfully */ }
    Err(ConfigError::Stale) => {
        // File was modified externally — reload before applying changes
        eprintln!("Config was edited externally, reloading");
    }
    Err(e) => return Err(e),
}
```

`LoadedConfig<T>` derefs to `T`, so you can read fields directly without unwrapping.

## Configuration precedence

1. `Default::default()` on your type
2. Values from the TOML file at `config_path(module)`
3. Environment variables with the given prefix (e.g. `APP_PORT=9000` sets `port`)

## Dependencies

- [`figment`](https://crates.io/crates/figment) — config merging
- [`libpath`](https://github.com/charliethomson/libpath) — config file path resolution
- [`libproduct`](https://github.com/charliethomson/libpath) — product name scoping
- [`serde`](https://crates.io/crates/serde) + [`toml`](https://crates.io/crates/toml) — serialization
