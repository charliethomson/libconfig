//! Demonstrates (and integration-tests) loading a containerized service's
//! config with libconfig: a deploy-controlled file (or no file at all), the
//! service's own `<PREFIX>_` vars, and fleet-shared unprefixed vars
//! (`AUTH_*`, `OTLP_ENDPOINT`, `PRODUCTION`, `SAMPLE_RATE`) — with no
//! OS-user-dir dependency and no forced `mkdir`.
//!
//! Run with `cargo run -p libconfig --example service`. It asserts its own
//! invariants, so a non-zero exit means a regression.

use libconfig::Loader;
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Default, PartialEq)]
struct ServiceConfig {
    /// Service-specific, set via the `SVC_` prefix.
    port: u16,
    /// Fleet-shared, unprefixed.
    auth_admin_key: String,
    otlp_endpoint: String,
    production: bool,
    sample_rate: f64,
}

const SHARED: [&str; 4] = ["AUTH_ADMIN_KEY", "OTLP_ENDPOINT", "PRODUCTION", "SAMPLE_RATE"];

fn main() {
    // SAFETY: single-threaded example; we set process env before reading it.
    unsafe {
        std::env::set_var("SVC_PORT", "8080");
        std::env::set_var("AUTH_ADMIN_KEY", "from-shared-env");
        std::env::set_var("OTLP_ENDPOINT", "http://otel:4317");
        std::env::set_var("PRODUCTION", "true");
        std::env::set_var("SAMPLE_RATE", "0.1");
    }

    // --- 1. Pure env (no file): defaults + shared env + prefixed env --------
    let cfg = Loader::pure_env()
        .env_prefix("SVC_")
        .shared_env(SHARED)
        .load::<ServiceConfig>()
        .expect("pure-env load");

    assert_eq!(
        cfg,
        ServiceConfig {
            port: 8080,
            auth_admin_key: "from-shared-env".into(),
            otlp_endpoint: "http://otel:4317".into(),
            production: true,
            sample_rate: 0.1,
        },
        "service must read both its own SVC_ vars and the bare shared vars"
    );

    // --- 2. Deploy-controlled file, with full precedence chain --------------
    // file < shared env < prefixed env.
    let dir = std::env::temp_dir().join("libconfig-service-example");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("service.toml");
    std::fs::write(
        &file,
        "port = 1\nsample_rate = 0.01\nauth_admin_key = \"from-file\"\notlp_endpoint = \"from-file\"\n",
    )
    .unwrap();

    // A prefixed override for sample_rate must beat the shared SAMPLE_RATE,
    // which in turn beats the file.
    unsafe {
        std::env::set_var("SVC_SAMPLE_RATE", "0.9");
    }

    let cfg = Loader::path(&file)
        .env_prefix("SVC_")
        .shared_env(SHARED)
        .load::<ServiceConfig>()
        .expect("path load");

    assert_eq!(cfg.port, 8080, "prefixed env overrides the file");
    assert!(
        (cfg.sample_rate - 0.9).abs() < 1e-9,
        "prefixed env beats shared env beats file (got {})",
        cfg.sample_rate
    );
    assert_eq!(
        cfg.auth_admin_key, "from-shared-env",
        "shared env overrides the file"
    );
    assert_eq!(cfg.otlp_endpoint, "http://otel:4317");

    // Loader::path is read-only by default: it must not rewrite the file.
    let on_disk = std::fs::read_to_string(&file).unwrap();
    assert!(
        on_disk.contains("from-file"),
        "Loader::path must not write back by default; file was: {on_disk}"
    );

    let _ = std::fs::remove_dir_all(&dir);

    println!("libconfig service example: all assertions passed");
}
