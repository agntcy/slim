// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Parse arbitrary strings as a SLIM name.
//!
//! Names are the addressing primitive, and `parse_name` is the only thing
//! between an arbitrary string and one: it takes operator input from config
//! files and `slimctl` arguments, and peer-supplied strings from control
//! messages.
//!
//! Invariant: no input panics, and a name that parses is complete. Both
//! `str_components` and `Display` `expect(...)` the string form to be
//! present, so a successful parse that left a component empty or unset is a
//! panic waiting to happen in a caller rather than here.

#![no_main]

use agntcy_slim_proto::dataplane::proto::v1::Name;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    // `parse_name` takes `&str`, so exercise UTF-8 boundaries too by letting
    // the fuzzer produce invalid sequences and discarding those.
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };

    if let Ok(name) = Name::parse_name(s) {
        // Panics if the string form was not populated.
        let (c0, c1, c2) = name.str_components();
        assert!(
            !c0.is_empty() && !c1.is_empty() && !c2.is_empty(),
            "parse_name accepted {s:?} but produced an empty component"
        );

        // Display must hold for anything parse_name accepts.
        let _ = name.to_string();
    }
});
