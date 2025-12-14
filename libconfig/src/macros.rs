/// Creates a lazily-initialized static configuration with optional environment variable overrides.
///
/// Generates a `LazyLock<T>` static that loads configuration from a TOML file on first access.
/// Configuration is loaded using figment, merging values in order:
/// 1. Type's `Default` implementation
/// 2. TOML file values (from `config_path(module)`)
/// 3. Environment variable overrides (if `env_prefix` specified)
///
/// # Options
///
/// - `module`: Required. Module name for config file path
/// - `env_prefix`: Optional. Prefix for environment variables (e.g., `"APP_"` enables `APP_SERVER_PORT=8080`)
/// - `impl_trait`: Optional. Implements `ConfigExt` trait, providing `load()` and `store()` methods
///
/// # Panics
///
/// Panics on first access if config file exists but cannot be parsed or doesn't match the type structure.
///
/// # Examples
///
/// ```rust
/// config! {
///     pub static APP_CONFIG: AppConfig = {
///         module: "myapp",
///     }
/// }
///
/// config! {
///     pub static DB_CONFIG: DatabaseConfig = {
///         module: "database",
///         env_prefix: "DB_",
///     }
/// }
///
/// config! {
///     pub static CACHE_CONFIG: CacheConfig = {
///         module: "cache",
///         impl_trait,
///     }
/// }
///
/// config! {
///     pub static FULL_CONFIG: FullConfig = {
///         module: "full",
///         env_prefix: "FULL_",
///         impl_trait,
///     }
/// }
/// ```
#[macro_export]
macro_rules! config {
    (
        $vis:vis static $name:ident: $ty:ty = {
            module: $module:literal,
            env_prefix: $env_prefix:literal,
            impl_trait $(,)?
        }
    ) => {
        $vis static $name: ::std::sync::LazyLock<$ty> = ::std::sync::LazyLock::new(|| {
            $crate::load::<$ty>($module, Some($env_prefix))
                .expect(concat!("Failed to load config for module: ", $module))
        });
        impl $crate::ConfigExt for $ty {
            fn module() -> &'static str {
                $module
            }
            fn env_prefix() -> Option<&'static str> {
                Some($env_prefix)
            }
        }
    };
    (
        $vis:vis static $name:ident: $ty:ty = {
            module: $module:literal,
            env_prefix: $env_prefix:literal $(,)?
        }
    ) => {
        $vis static $name: ::std::sync::LazyLock<$ty> = ::std::sync::LazyLock::new(|| {
            $crate::load::<$ty>($module, Some($env_prefix))
                .expect(concat!("Failed to load config for module: ", $module))
        });
    };
    (
        $vis:vis static $name:ident: $ty:ty = {
            module: $module:literal,
            impl_trait $(,)?
        }
    ) => {
        $vis static $name: ::std::sync::LazyLock<$ty> = ::std::sync::LazyLock::new(|| {
            $crate::load::<$ty>($module, None)
                .expect(concat!("Failed to load config for module: ", $module))
        });
        impl $crate::ConfigExt for $ty {
            fn module() -> &'static str {
                $module
            }
            fn env_prefix() -> Option<&'static str> {
                None
            }
        }
    };
    (
        $vis:vis static $name:ident: $ty:ty = {
            module: $module:literal $(,)?
        }
    ) => {
        $vis static $name: ::std::sync::LazyLock<$ty> = ::std::sync::LazyLock::new(|| {
            $crate::load::<$ty>($module, None)
                .expect(concat!("Failed to load config for module: ", $module))
        });
    };
}
