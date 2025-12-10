use libproduct::product_name;
use serde::{Deserialize, Serialize};

product_name!("dev.thmsn.libconfig");

#[derive(Serialize, Deserialize, Default, Debug)]
struct SampleConfig {
    loads: usize,
}

#[tokio::main]
async fn main() {
    PRODUCT_NAME.set_global().unwrap();

    // Reads/writes to Application Support/dev.thmsn.libconfig/configs/testing.toml
    let config = libconfig::merge_config("testing", |config: &mut SampleConfig| {
        config.loads = config.loads.saturating_add(1);
    })
    .unwrap();

    println!("{:#?}", config);
}
