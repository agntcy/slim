// The schema generator depends on the native-only `ServerConfig` and filesystem
// access; it is meaningless in a browser, so on wasm32 it compiles to a no-op.
cfg_if::cfg_if! {
    if #[cfg(not(target_arch = "wasm32"))] {
        use schemars::{schema_for, JsonSchema};
        use slim_config::client::ClientConfig;
        use slim_config::server::ServerConfig;
        use std::fs::File;
        use std::io::Write;
        use std::path::PathBuf;

        fn write_schema<T: JsonSchema>(file_name: &str) {
            let schema = schema_for!(T);
            let schema_json = serde_json::to_string_pretty(&schema).unwrap();

            let mut path = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
            path.push(format!("src/schema/{}", file_name));

            let mut file = File::create(&path).unwrap();
            file.write_all(schema_json.as_bytes()).unwrap();
            println!("Schema written to {:?}", path);
        }

        fn main() {
            write_schema::<ClientConfig>("client-config.schema.json");
            write_schema::<ServerConfig>("server-config.schema.json");
        }
    } else {
        fn main() {}
    }
}
