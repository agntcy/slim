// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Parse arbitrary strings as a `slimctl` route.
//!
//! `parse_route` turns operator input of the form
//! `org/namespace/agentname[/agentid]` into name components.
//!
//! Invariant: no input panics (including too few or too many `/`, empty
//! parts, and multi-byte UTF-8 next to separators), and a route that parses
//! has exactly 2 or 3 separators and three non-empty components that
//! contain no `/`.

#![no_main]

use agntcy_slimctl::utils::parse_route;
use libfuzzer_sys::fuzz_target;

/// `NameId::NULL_COMPONENT`, the agent id used when the route omits one.
const NULL_COMPONENT: u128 = u128::MAX;

fuzz_target!(|data: &[u8]| {
    // `parse_route` takes `&str`, so exercise UTF-8 boundaries by letting
    // the fuzzer produce invalid sequences and discarding those.
    let Ok(s) = std::str::from_utf8(data) else {
        return;
    };

    let Ok((org, ns, agent_type, agent_id)) = parse_route(s) else {
        return;
    };

    let separators = s.matches('/').count();
    assert!(
        separators == 2 || separators == 3,
        "parse_route accepted {s:?} with {separators} separators"
    );

    for part in [&org, &ns, &agent_type] {
        assert!(
            !part.is_empty() && !part.contains('/'),
            "parse_route accepted {s:?} but produced component {part:?}"
        );
    }

    let mut parts = s.split('/');
    assert_eq!(parts.next(), Some(org.as_str()));
    assert_eq!(parts.next(), Some(ns.as_str()));
    assert_eq!(parts.next(), Some(agent_type.as_str()));

    if separators == 2 {
        assert_eq!(
            agent_id, NULL_COMPONENT,
            "omitted agent id in {s:?} must default to NULL_COMPONENT"
        );
    }
});
