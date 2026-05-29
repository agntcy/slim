// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use anyhow::{Context, Result};
use prost::Message;
use prost_types::compiler::{CodeGeneratorRequest, CodeGeneratorResponse};
use std::collections::HashMap;
use std::io::{self, Read, Write};

/// Read a CodeGeneratorRequest from stdin
pub fn read_request() -> Result<CodeGeneratorRequest> {
    let mut buf = Vec::new();
    io::stdin()
        .read_to_end(&mut buf)
        .context("Failed to read from stdin")?;
    CodeGeneratorRequest::decode(&buf[..]).context("Failed to decode CodeGeneratorRequest")
}

/// Write a CodeGeneratorResponse to stdout
pub fn write_response(response: &CodeGeneratorResponse) -> Result<()> {
    let mut buf = Vec::new();
    response
        .encode(&mut buf)
        .context("Failed to encode CodeGeneratorResponse")?;
    io::stdout()
        .write_all(&buf)
        .context("Failed to write to stdout")?;
    Ok(())
}

/// Parse parameters from the parameter string
/// Format: "key1=value1,key2=value2"
pub fn parse_parameters(param_str: &str) -> HashMap<String, String> {
    let mut params = HashMap::new();

    if param_str.is_empty() {
        return params;
    }

    for pair in param_str.split(',') {
        if let Some((key, value)) = pair.split_once('=') {
            params.insert(key.to_string(), value.to_string());
        }
    }

    params
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_parameters_empty() {
        let params = parse_parameters("");
        assert!(params.is_empty());
    }

    #[test]
    fn test_parse_parameters_single() {
        let params = parse_parameters("key=value");
        assert_eq!(params.get("key"), Some(&"value".to_string()));
    }

    #[test]
    fn test_parse_parameters_multiple() {
        let params = parse_parameters("types_import=from . import types,other=value");
        assert_eq!(
            params.get("types_import"),
            Some(&"from . import types".to_string())
        );
        assert_eq!(params.get("other"), Some(&"value".to_string()));
    }
}
