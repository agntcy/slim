use schemars::{JsonSchema, schema_for};
use slim_config::grpc::client::ClientConfig;
use slim_config::grpc::server::ServerConfig;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

fn write_schema<T: JsonSchema>(file_name: &str) {
    let schema = schema_for!(T);
    let schema_json = serde_json::to_string_pretty(&schema).unwrap();

    let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    path.push(format!("src/grpc/schema/{}", file_name));

    let mut file = File::create(&path).unwrap();
    file.write_all(schema_json.as_bytes()).unwrap();
    println!("Schema written to {:?}", path);
}

fn main() {
    write_schema::<ClientConfig>("client-config.schema.json");
    write_schema::<ServerConfig>("server-config.schema.json");
}
