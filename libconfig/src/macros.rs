/// Creates a lazily-initialized static configuration with optional environment variable overrides.
///
/// Generates a `LazyLock<T>` static that loads configuration from a TOML file on first access.
/// Configuration is loaded via [`Loader`](crate::Loader), merging values in increasing precedence:
/// 1. Type's `Default` implementation
/// 2. TOML file values (from `config_path(module)`)
/// 3. Bare **shared** env keys (unprefixed, if `shared_env` specified)
/// 4. Prefixed environment variables (if `env_prefix` specified)
///
/// # Options
///
/// - `module`: Required. Module name for config file path
/// - `env_prefix`: Optional. Prefix for the highest-precedence env layer (e.g., `"APP_"` enables `APP_SERVER_PORT=8080`)
/// - `shared_env`: Optional. A list of unprefixed, fleet-shared env keys to merge between the
///   TOML file and the prefixed-env layer (e.g., `["AUTH_ADMIN_KEY", "OTLP_ENDPOINT"]`)
/// - `impl_trait`: Optional. Implements `ConfigExt` trait, providing `load()` and `store()` methods
///
/// Fields must appear in the order shown: `module`, then `env_prefix`, then `shared_env`, then
/// `impl_trait`. Each after `module` is optional.
///
/// # Panics
///
/// Panics on first access if config file exists but cannot be parsed or doesn't match the type structure.
///
/// # Examples
///
/// ```rust
/// # use libconfig::{config, ConfigExt};
/// # use serde::{Deserialize, Serialize};
/// # #[derive(Serialize, Deserialize, Default)]
/// # struct AppConfig { port: u16 }
/// # #[derive(Serialize, Deserialize, Default)]
/// # struct ServiceConfig { sample_rate: f64 }
/// // Basic: defaults + TOML file only.
/// config! {
///     pub static APP_CONFIG: AppConfig = {
///         module: "myapp",
///     }
/// }
///
/// // A containerized service: its own prefix plus fleet-shared, unprefixed vars.
/// config! {
///     pub static SVC_CONFIG: ServiceConfig = {
///         module: "service",
///         env_prefix: "SVC_",
///         shared_env: ["AUTH_ADMIN_KEY", "OTLP_ENDPOINT", "PRODUCTION", "SAMPLE_RATE"],
///         impl_trait,
///     }
/// }
/// # fn main() {}
/// ```
#[macro_export]
macro_rules! config {
    // With `impl_trait`.
    (
        $vis:vis static $name:ident: $ty:ty = {
            module: $module:literal
            $(, env_prefix: $env_prefix:literal)?
            $(, shared_env: [ $($shared:literal),* $(,)? ])?
            , impl_trait $(,)?
        }
    ) => {
        $vis static $name: ::std::sync::LazyLock<$ty> = ::std::sync::LazyLock::new(|| {
            $crate::__config_load!($ty, $module $(, env_prefix: $env_prefix)? $(, shared_env: [$($shared),*])?)
        });
        impl $crate::ConfigExt for $ty {
            fn module() -> &'static str {
                $module
            }
            fn env_prefix() -> Option<&'static str> {
                #[allow(unused_mut)]
                let mut prefix: ::std::option::Option<&'static str> = ::std::option::Option::None;
                $( prefix = ::std::option::Option::Some($env_prefix); )?
                prefix
            }
            fn shared_env() -> &'static [&'static str] {
                &[ $($($shared),*)? ]
            }
        }
    };
    // Without `impl_trait`.
    (
        $vis:vis static $name:ident: $ty:ty = {
            module: $module:literal
            $(, env_prefix: $env_prefix:literal)?
            $(, shared_env: [ $($shared:literal),* $(,)? ])?
            $(,)?
        }
    ) => {
        $vis static $name: ::std::sync::LazyLock<$ty> = ::std::sync::LazyLock::new(|| {
            $crate::__config_load!($ty, $module $(, env_prefix: $env_prefix)? $(, shared_env: [$($shared),*])?)
        });
    };
}

/// Internal: build a [`Loader`](crate::Loader) for `module` with the given
/// optional `env_prefix` / `shared_env`, load it, and `expect` on failure.
#[doc(hidden)]
#[macro_export]
macro_rules! __config_load {
    ($ty:ty, $module:literal $(, env_prefix: $env_prefix:literal)? $(, shared_env: [ $($shared:literal),* ])?) => {{
        #[allow(unused_mut)]
        let mut loader = $crate::Loader::module($module);
        $( loader = loader.env_prefix($env_prefix); )?
        $( loader = loader.shared_env([ $($shared),* ]); )?
        loader
            .load::<$ty>()
            .expect(concat!("Failed to load config for module: ", $module))
    }};
}
