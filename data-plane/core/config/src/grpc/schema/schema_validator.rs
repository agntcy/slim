use jsonschema::Validator;
use serde_json::Value;
use std::fs;
use std::path::Path;

#[derive(Debug)]
pub struct SchemaValidator {
    schema_validator: Validator,
}

impl SchemaValidator {
    pub fn new<P: AsRef<Path>>(schema_path: P) -> Result<Self, String> {
        let schema_str = fs::read_to_string(schema_path)
            .map_err(|e| format!("Failed to read schema file: {}", e))?;
        let schema_json: Value = serde_json::from_str(&schema_str)
            .map_err(|e| format!("Failed to parse schema JSON: {}", e))?;
        let schema_validator = jsonschema::validator_for(&schema_json)
            .map_err(|e| format!("Failed to compile schema: {}", e))?;
        Ok(SchemaValidator { schema_validator })
    }

    pub fn is_valid(&self, data: &str) -> bool {
        match serde_json::from_str(data) {
            Ok(json_data) => self.schema_validator.is_valid(&json_data),
            Err(_) => false,
        }
    }
}

impl Default for SchemaValidator {
    fn default() -> Self {
        let schema = serde_json::json!({});
        let schema_validator = Validator::new(&schema).expect("Failed to create default Validator");
        SchemaValidator { schema_validator }
    }
}