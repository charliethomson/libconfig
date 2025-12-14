use libconfig::{ConfigExt, config};
use libproduct::product_name;
use serde::{Deserialize, Serialize};

product_name!(with base "dev.thmsn.libconfig" as PRODUCT_NAME);

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
struct SampleConfig {
    loads: usize,
}
config! {
    pub static SAMPLE_CONFIG: SampleConfig = {
        module: "sample",
        env_prefix: "TESTING_",
        impl_trait,
    }
}

#[tokio::main]
async fn main() {
    PRODUCT_NAME.set_global().unwrap();
    let mut conf = SAMPLE_CONFIG.clone();

    println!("Loaded as {:#?}", conf);

    conf.loads += 1;
    conf.store().unwrap();

    println!("Stored as {:#?}", conf);
}
