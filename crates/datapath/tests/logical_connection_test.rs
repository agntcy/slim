// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Acceptance tests for logical connections, exercised through the real
//! `Forwarder` and subscription table rather than the logical connection table
//! in isolation.
//!
//! These cover the behaviours the feature exists for:
//! * several physical connections to one peer look like a single connection to
//!   the subscription and forwarding tables;
//! * the same subscription arriving on two sub-connections produces one entry;
//! * a message received on any sub-connection is never forwarded back out on a
//!   sibling sub-connection (the routing loop this feature fixes);
//! * a connection that was never grouped behaves exactly as it did before.

use slim_datapath::api::{EncodedName, ProtoName as Name};
use slim_datapath::errors::DataPathError;
use slim_datapath::forwarder::Forwarder;
use slim_datapath::logical_connection::{SubConnPolicy, is_logical};
use slim_datapath::tables::{ConnType, MatchFilter};

/// A forwarder whose connection payloads are plain integers — the tests only
/// care about routing decisions, not about transports.
fn forwarder() -> Forwarder<u32> {
    Forwarder::<u32>::new()
}

fn multicast() -> Name {
    Name::from_strings(["agntcy", "default", "multicast"])
}

fn enc(name: &Name) -> EncodedName {
    name.name.unwrap()
}

/// Criterion 1: K physical connections expose a single logical connection.
#[test]
fn sub_connections_of_one_peer_share_a_single_routing_id() {
    let fwd = forwarder();

    let l1 = fwd.attach_sub_connection(1, "peer-a", SubConnPolicy::RoundRobin);
    let l2 = fwd.attach_sub_connection(2, "peer-a", SubConnPolicy::RoundRobin);
    let l3 = fwd.attach_sub_connection(3, "peer-a", SubConnPolicy::RoundRobin);

    assert_eq!(
        l1, l2,
        "sub-connections of one peer must share a logical id"
    );
    assert_eq!(l2, l3);
    assert!(is_logical(l1), "grouped connections get a logical id");

    for phys in [1, 2, 3] {
        assert_eq!(fwd.routing_id(phys), l1);
    }

    // A different peer is a different logical connection.
    let other = fwd.attach_sub_connection(4, "peer-b", SubConnPolicy::RoundRobin);
    assert_ne!(other, l1);
}

/// Criterion 2: the same name subscribed on different sub-connections yields a
/// single subscription table entry.
#[test]
fn same_subscription_on_two_sub_connections_is_one_entry() {
    let fwd = forwarder();
    let name = multicast();

    fwd.attach_sub_connection(1, "peer-a", SubConnPolicy::RoundRobin);
    fwd.attach_sub_connection(2, "peer-a", SubConnPolicy::RoundRobin);

    // App A subscribes; its subscription travels over sub-connection 1.
    let created = fwd
        .on_subscription_msg(name.clone(), 1, ConnType::Remote, true, 1)
        .expect("first subscription must be accepted");
    assert!(created, "the first subscription creates the entry");

    // App B subscribes to the same name; its subscription travels over
    // sub-connection 2. Before logical connections this added a second,
    // independent entry — which is what closed the routing loop.
    let created = fwd
        .on_subscription_msg(name.clone(), 2, ConnType::Remote, true, 2)
        .expect("second subscription must be accepted");
    assert!(
        !created,
        "a subscription arriving on a sibling sub-connection must not create a second entry",
    );

    // A publish from an unrelated connection resolves to exactly one target.
    let targets = fwd
        .on_publish_msg_match(enc(&name), 99, u32::MAX, MatchFilter::ALL)
        .expect("the subscription must be matchable");
    assert_eq!(
        targets.len(),
        1,
        "one peer must appear once in the match result, got {targets:?}",
    );
    assert_eq!(targets[0], fwd.routing_id(1));
}

/// Criterion 3: a message received on any sub-connection of L is never
/// forwarded back to any sub-connection of L. This is the routing loop from the
/// issue, reproduced directly.
#[test]
fn message_is_not_forwarded_back_to_a_sibling_sub_connection() {
    let fwd = forwarder();
    let name = multicast();

    fwd.attach_sub_connection(1, "peer-a", SubConnPolicy::RoundRobin);
    fwd.attach_sub_connection(2, "peer-a", SubConnPolicy::RoundRobin);

    fwd.on_subscription_msg(name.clone(), 1, ConnType::Remote, true, 1)
        .unwrap();
    fwd.on_subscription_msg(name.clone(), 2, ConnType::Remote, true, 2)
        .unwrap();

    // A message arriving on sub-connection 1 must find no target: the only
    // subscriber is the peer it came from. Previously sub-connection 2 was a
    // valid target and the message went straight back to the same peer.
    for incoming in [1, 2] {
        let res = fwd.on_publish_msg_match(enc(&name), incoming, u32::MAX, MatchFilter::ALL);
        assert!(
            matches!(res, Err(DataPathError::NoMatchEncoded(..))),
            "a message from sub-connection {incoming} must not be forwarded back to its peer, \
             got {res:?}",
        );
    }
}

/// The exclusion must not be over-broad: a genuinely different peer subscribed
/// to the same name still receives the message.
#[test]
fn other_peers_still_receive_messages_from_an_excluded_peer() {
    let fwd = forwarder();
    let name = multicast();

    fwd.attach_sub_connection(1, "peer-a", SubConnPolicy::RoundRobin);
    fwd.attach_sub_connection(2, "peer-a", SubConnPolicy::RoundRobin);
    let peer_b = fwd.attach_sub_connection(3, "peer-b", SubConnPolicy::RoundRobin);

    fwd.on_subscription_msg(name.clone(), 1, ConnType::Remote, true, 1)
        .unwrap();
    fwd.on_subscription_msg(name.clone(), 2, ConnType::Remote, true, 2)
        .unwrap();
    fwd.on_subscription_msg(name.clone(), 3, ConnType::Remote, true, 3)
        .unwrap();

    let targets = fwd
        .on_publish_msg_match(enc(&name), 1, u32::MAX, MatchFilter::ALL)
        .expect("peer-b must still match");
    assert_eq!(
        targets,
        vec![peer_b],
        "only the sending peer is excluded, not everyone else",
    );
}

/// Criterion 5: a connection that was never attached to a group routes exactly
/// as before — its routing id is its own physical id, and sends address it
/// directly.
#[test]
fn ungrouped_connections_are_unchanged() {
    let fwd = forwarder();
    let name = multicast();

    assert_eq!(
        fwd.routing_id(7),
        7,
        "an ungrouped id is its own routing id"
    );
    assert!(!is_logical(7));
    assert_eq!(fwd.select_sub_connections(7, None), vec![7]);

    fwd.on_subscription_msg(name.clone(), 7, ConnType::Remote, true, 1)
        .unwrap();

    let targets = fwd
        .on_publish_msg_match(enc(&name), 8, u32::MAX, MatchFilter::ALL)
        .unwrap();
    assert_eq!(targets, vec![7]);

    // And it is still excluded when it is the sender.
    let res = fwd.on_publish_msg_match(enc(&name), 7, u32::MAX, MatchFilter::ALL);
    assert!(matches!(res, Err(DataPathError::NoMatchEncoded(..))));
}

/// Criterion 4: the send path expands a logical connection according to the
/// configured policy.
#[test]
fn policy_controls_which_sub_connections_a_send_uses() {
    // Round robin spreads consecutive sends across all sub-connections.
    let fwd = forwarder();
    let rr = fwd.attach_sub_connection(1, "peer-a", SubConnPolicy::RoundRobin);
    fwd.attach_sub_connection(2, "peer-a", SubConnPolicy::RoundRobin);
    fwd.attach_sub_connection(3, "peer-a", SubConnPolicy::RoundRobin);

    let picks: Vec<u64> = (0..6)
        .map(|_| {
            let sel = fwd.select_sub_connections(rr, None);
            assert_eq!(sel.len(), 1, "round robin sends one copy");
            sel[0]
        })
        .collect();
    assert_eq!(picks, vec![1, 2, 3, 1, 2, 3]);

    // Redundant sends one copy on every sub-connection.
    let fwd = forwarder();
    let red = fwd.attach_sub_connection(1, "peer-b", SubConnPolicy::Redundant);
    fwd.attach_sub_connection(2, "peer-b", SubConnPolicy::Redundant);
    fwd.attach_sub_connection(3, "peer-b", SubConnPolicy::Redundant);

    let mut all = fwd.select_sub_connections(red, None);
    all.sort_unstable();
    assert_eq!(
        all,
        vec![1, 2, 3],
        "redundant duplicates onto every sub-conn"
    );
}

/// Dropping one sub-connection must not tear down the peer's subscriptions,
/// because the surviving sub-connection still reaches it. Dropping the last one
/// must.
#[test]
fn subscriptions_survive_until_the_last_sub_connection_drops() {
    let fwd = forwarder();
    let name = multicast();

    fwd.attach_sub_connection(1, "peer-a", SubConnPolicy::RoundRobin);
    fwd.attach_sub_connection(2, "peer-a", SubConnPolicy::RoundRobin);
    fwd.on_subscription_msg(name.clone(), 1, ConnType::Remote, true, 1)
        .unwrap();

    // Sub-connection 1 dies. Nothing is withdrawn, and the route still matches
    // through the surviving sibling.
    let removed = fwd.on_connection_drop(1, ConnType::Remote);
    assert!(
        removed.is_empty(),
        "no subscriptions are withdrawn while a sibling survives, got {removed:?}",
    );

    let logical = fwd.routing_id(2);
    let targets = fwd
        .on_publish_msg_match(enc(&name), 99, u32::MAX, MatchFilter::ALL)
        .expect("the route must survive the loss of one sub-connection");
    assert_eq!(targets, vec![logical]);
    assert_eq!(
        fwd.select_sub_connections(logical, None),
        vec![2],
        "traffic moves to the surviving sub-connection",
    );

    // The last sub-connection dies: now the subscription is withdrawn.
    let removed = fwd.on_connection_drop(2, ConnType::Remote);
    assert!(
        !removed.is_empty(),
        "losing the last sub-connection must withdraw the peer's subscriptions",
    );
    let res = fwd.on_publish_msg_match(enc(&name), 99, u32::MAX, MatchFilter::ALL);
    assert!(
        matches!(res, Err(DataPathError::NoMatchEncoded(..))),
        "the route must be gone, got {res:?}",
    );
}

/// Criterion 6 in miniature: many subscribers reachable through one intermediate
/// peer that holds several sub-connections. A message from that peer fans out to
/// the local subscribers only, and never back to the peer — so no amount of
/// sub-connections produces a loop.
#[test]
fn fan_out_through_an_intermediate_peer_terminates() {
    let fwd = forwarder();
    let name = multicast();

    // The intermediate peer is reached over 3 sub-connections.
    for phys in 1..=3u64 {
        fwd.attach_sub_connection(phys, "intermediate", SubConnPolicy::RoundRobin);
    }
    // Its subscription arrives on all three, as would happen when 3 apps on the
    // far side each subscribe over their own connection.
    for (i, phys) in (1..=3u64).enumerate() {
        fwd.on_subscription_msg(name.clone(), phys, ConnType::Remote, true, i as u64 + 1)
            .unwrap();
    }

    // 10 local subscribers.
    let locals: Vec<u64> = (100..110).collect();
    for (i, &conn) in locals.iter().enumerate() {
        fwd.on_subscription_msg(name.clone(), conn, ConnType::Local, true, 100 + i as u64)
            .unwrap();
    }

    // A message from the intermediate peer reaches every local subscriber and
    // nothing else.
    let mut targets = fwd
        .on_publish_msg_match(enc(&name), 2, u32::MAX, MatchFilter::ALL)
        .expect("local subscribers must match");
    targets.sort_unstable();
    assert_eq!(
        targets, locals,
        "a message from the peer must reach exactly the 10 local subscribers",
    );

    // A message from a local subscriber reaches the peer exactly once, plus the
    // other 9 locals — not once per sub-connection.
    let targets = fwd
        .on_publish_msg_match(enc(&name), locals[0], u32::MAX, MatchFilter::ALL)
        .expect("the peer must match");
    let peer_hits = targets.iter().filter(|&&t| t == fwd.routing_id(1)).count();
    assert_eq!(
        peer_hits, 1,
        "the peer must be targeted once regardless of sub-connection count, got {targets:?}",
    );
    assert_eq!(targets.len(), locals.len(), "9 other locals plus the peer");
}
