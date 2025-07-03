use schemars::schema_for;
use slim_config::grpc::client::ClientConfig;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn main() {
    let schema = schema_for!(ClientConfig);
    let schema_json = serde_json::to_string_pretty(&schema).unwrap();

    // Write to the same directory as this script
    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push("src/grpc/schema/client-config.schema.json");

    let mut file = File::create(&path).unwrap();
    file.write_all(schema_json.as_bytes()).unwrap();
    println!("Schema written to {:?}", path);
}
