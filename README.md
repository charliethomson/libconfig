# libconfig

A Rust library for loading, storing, and managing application configuration backed by TOML files with optional environment variable overrides.

## Features

- Loads config from a TOML file resolved via [`libpath`](https://github.com/charliethomson/libpath)
- Merges values in priority order: type defaults → TOML file → bare shared env → prefixed env
- A bare **shared-env** layer for fleet-wide, unprefixed vars (`AUTH_ADMIN_KEY`, `OTLP_ENDPOINT`, …)
- Pluggable file source: OS config dir, an explicit deploy path, or no file at all (pure env)
- Container-friendly: no OS-user-dir dependency and no forced `mkdir` (via `Loader::path` / `Loader::pure_env`)
- Writes atomically (temp file + `rename(2)`) to avoid partial writes
- Recovers from corrupt config files by falling back to defaults (write-back sources only)
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

// Containerized service: its own APP_ vars plus fleet-shared, unprefixed vars.
config! {
    pub static APP_CONFIG: AppConfig = {
        module: "myapp",
        env_prefix: "APP_",
        shared_env: ["AUTH_ADMIN_KEY", "OTLP_ENDPOINT", "PRODUCTION", "SAMPLE_RATE"],
        impl_trait,
    }
}
```

Macro fields must appear in this order: `module`, then optional `env_prefix`,
`shared_env`, and `impl_trait`.

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

### Containerized services with `Loader`

Desktop tools use the OS config dir and write changes back. Services typically
read a deploy-controlled file (or pure env), in a container, and must not depend
on an OS-user dir or create directories. The `Loader` builder covers both:

```rust
use libconfig::Loader;

// Read a deploy-controlled file + the service's own SVC_ vars + fleet-shared,
// unprefixed vars. Never creates directories; read-only by default.
let cfg = Loader::path("/etc/myproduct/config.toml")
    .env_prefix("SVC_")
    .shared_env(["AUTH_ADMIN_KEY", "AUTH_TCP_ADDR", "OTLP_ENDPOINT", "PRODUCTION", "SAMPLE_RATE"])
    .load::<ServiceConfig>()?;

// No file at all — pure defaults + env.
let cfg = Loader::pure_env()
    .env_prefix("SVC_")
    .shared_env(["AUTH_ADMIN_KEY", "OTLP_ENDPOINT"])
    .load::<ServiceConfig>()?;
```

- `Loader::module(m)` — OS config dir via libpath, writes back (the desktop default).
- `Loader::path(p)` — explicit file, no `mkdir`, read-only unless `.write_back(true)`.
- `Loader::pure_env()` — no file; never touches the filesystem.

`shared_env` keys are matched case-insensitively and map to the lower-cased
config field of the same name (`OTLP_ENDPOINT` → `otlp_endpoint`). To redirect a
service's libpath-resolved roots under a mount instead of passing an explicit
path, set [`libpath::set_base_override`](https://github.com/charliethomson/libpath)
(or `LIBPATH_BASE_DIR`) and `libpath::set_create_dirs(false)`. See
[`examples/service.rs`](libconfig/examples/service.rs) for a worked example that
doubles as an integration test.

## Configuration precedence

1. `Default::default()` on your type
2. Values from the TOML file (`config_path(module)`, an explicit path, or none)
3. Bare **shared** env — the unprefixed keys named via `shared_env` (e.g. `OTLP_ENDPOINT`)
4. Prefixed environment variables (e.g. `APP_PORT=9000` sets `port`)

## Dependencies

- [`figment`](https://crates.io/crates/figment) — config merging
- [`libpath`](https://github.com/charliethomson/libpath) — config file path resolution
- [`libproduct`](https://github.com/charliethomson/libpath) — product name scoping
- [`serde`](https://crates.io/crates/serde) + [`toml`](https://crates.io/crates/toml) — serialization
