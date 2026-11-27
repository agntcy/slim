// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Parse arbitrary strings as a `slimctl` duration.
//!
//! `parse_duration` handles `--timeout` and `slimctl config set timeout`,
//! whose result is written to the config file and parsed again on every
//! later command.
//!
//! Invariants:
//! - no input panics;
//! - a value too large for the parser is rejected, never wrapped or saturated;
//! - a duration that parses survives being written back as `DurationString`
//!   and parsed again, so `config set timeout` cannot leave an unreadable file.

#![no_main]

use agntcy_slimctl::config::parse_duration;
use duration_string::DurationString;
use libfuzzer_sys::arbitrary::{Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;

const UNITS: [(&str, u128); 9] = [
    ("ns", 1),
    ("us", 1_000),
    ("ms", 1_000_000),
    ("s", 1_000_000_000),
    ("m", 60_000_000_000),
    ("h", 3_600_000_000_000),
    ("d", 86_400_000_000_000),
    ("w", 604_800_000_000_000),
    ("y", 31_556_926_000_000_000),
];

/// Largest duration `parse_duration` accepts.
const MAX_NANOS: u128 = u64::MAX as u128;

fuzz_target!(|data: &[u8]| {
    // Raw strings: no panic, and whatever parses must round-trip.
    if let Ok(s) = std::str::from_utf8(data) {
        if let Ok(d) = parse_duration(s) {
            let written = DurationString::from(d).to_string();
            assert_eq!(
                parse_duration(&written).ok(),
                Some(d),
                "{s:?} parsed to {d:?} but was written as {written:?}, which does not parse back"
            );
        }
    }

    // Structured input: large numbers with real units, checked against
    // a total computed independently in u128.
    let mut u = Unstructured::new(data);
    let Ok(groups) = Vec::<(u64, u8)>::arbitrary(&mut u) else {
        return;
    };
    if groups.is_empty() {
        return;
    }

    let mut input = String::new();
    let mut expected = Some(0u128);
    for (value, unit) in groups {
        let (suffix, nanos) = UNITS[usize::from(unit) % UNITS.len()];
        input.push_str(&format!("{value}{suffix}"));
        expected = expected.and_then(|t| t.checked_add(u128::from(value) * nanos));
    }
    let expected = expected.filter(|n| *n <= MAX_NANOS);

    match (parse_duration(&input), expected) {
        (Ok(d), Some(n)) => assert_eq!(d.as_nanos(), n, "{input:?} parsed to the wrong value"),
        (Err(_), None) => {}
        (got, want) => panic!("{input:?}: got {got:?}, expected {want:?} nanoseconds"),
    }
});
