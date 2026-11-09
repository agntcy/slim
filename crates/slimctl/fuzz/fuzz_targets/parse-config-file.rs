// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Parse arbitrary bytes as the contents of a `slimctl` connection config file.
//!
//! `parse_config_file` takes a path, so fuzzing the path string would only
//! ever exercise `open()` and "file not found". This target fuzzes the file
//! contents instead, through `parse_config_data` and
//! `parse_config_data_with_link_id`, which are the same parsers without the
//! file read in front.
//!
//! Invariants:
//! - no input panics;
//! - an accepted config is passed through unchanged as `config_data`, and its
//!   endpoint becomes the connection's `link_id`;
//! - `parse_config_data_with_link_id` always returns a non-empty link id, and
//!   the `config_data` it produces is itself a valid config carrying that
//!   link id and the same endpoint, since the node registers the connection
//!   from it.

#![no_main]

use agntcy_slimctl::utils::{parse_config_data, parse_config_data_with_link_id};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // Config files are read with `read_to_string`, which rejects invalid
    // UTF-8 before the parser runs.
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };

    if let Ok(conn) = parse_config_data(s.to_string()) {
        assert_eq!(
            conn.config_data, s,
            "accepted config was not passed through unchanged"
        );
        assert!(
            !conn.link_id.is_empty(),
            "accepted {s:?} with an empty endpoint"
        );
    }

    let Ok((conn, link_id)) = parse_config_data_with_link_id(s) else {
        return;
    };
    assert!(
        !link_id.trim().is_empty(),
        "{s:?} produced an empty link id"
    );

    let reparsed: serde_json::Value = serde_json::from_str(&conn.config_data)
        .unwrap_or_else(|e| panic!("{s:?} produced config_data that is not JSON: {e}"));
    assert_eq!(
        reparsed.get("link_id").and_then(|v| v.as_str()),
        Some(link_id.as_str()),
        "{s:?} produced config_data without its link id"
    );

    let (again, again_link_id) = parse_config_data_with_link_id(&conn.config_data)
        .unwrap_or_else(|e| panic!("{s:?} produced config_data that does not parse back: {e}"));
    assert_eq!(again_link_id, link_id, "{s:?}: link id changed on reparse");
    assert_eq!(
        again.link_id, conn.link_id,
        "{s:?}: endpoint changed on reparse"
    );
});
