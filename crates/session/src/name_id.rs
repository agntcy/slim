// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! The name ids SLIM derives from identities, so a receiver can recompute
//! them from a peer's verified `sub`.

use slim_datapath::api::{NameId, ProtoName};

/// Salts tried for a channel's ids before giving up.
const MAX_CHANNEL_SALT: u64 = 10;

/// The name id of an app whose identity (`TokenProvider::get_id`) is `sub`.
pub fn app_id(sub: &str) -> u128 {
    let mut id = twox_hash::XxHash3_128::oneshot(sub.as_bytes());
    if NameId::is_reserved_id(id) {
        id %= u128::MAX - NameId::RESERVED_IDS;
    }
    id
}

/// The data and control ids of the multicast channel `org/ns/name` created
/// by `sub`. `None` if no salt gives two distinct ids, which doesn't happen
/// in practice.
pub fn channel_ids(sub: &str, org: &str, ns: &str, name: &str) -> Option<(u128, u128)> {
    let data_input = format!("{sub}/{org}/{ns}/{name}/data");
    let ctrl_input = format!("{sub}/{org}/{ns}/{name}/control");
    let fold = |id: u128| {
        if NameId::is_reserved_id(id) {
            (id % (u128::MAX - NameId::RESERVED_IDS - 1)) + 1
        } else {
            id
        }
    };
    (0..=MAX_CHANNEL_SALT).find_map(|salt| {
        let data = fold(twox_hash::XxHash3_128::oneshot_with_seed(
            salt,
            data_input.as_bytes(),
        ));
        let ctrl = fold(twox_hash::XxHash3_128::oneshot_with_seed(
            salt,
            ctrl_input.as_bytes(),
        ));
        (data != ctrl).then_some((data, ctrl))
    })
}

/// Whether `channel` and `control` are the channel names `sub` derives: same
/// readable name, with the data and control ids of [`channel_ids`]. The
/// readable components are checked against the encoded ones the node routes
/// on, so the strings can't name one channel while the name routes to another.
pub fn is_channel_of(sub: &str, channel: &ProtoName, control: &ProtoName) -> bool {
    let Some(strings) = channel.str_name.as_ref() else {
        return false;
    };
    let (org, ns, name) = (
        strings.str_component_0.as_str(),
        strings.str_component_1.as_str(),
        strings.str_component_2.as_str(),
    );
    let readable = ProtoName::from_strings([org, ns, name]);
    if channel.name.is_none()
        || control.name.is_none()
        || !readable.match_prefix(channel)
        || !readable.match_prefix(control)
    {
        return false;
    }
    channel_ids(sub, org, ns, name)
        .is_some_and(|(data, ctrl)| channel.id() == data && control.id() == ctrl)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_id_is_the_xxh3_of_the_sub() {
        // A known derivation: this did:key's name id, as in agntcy/slim#2264.
        let did = "did:key:z6Mkfvtar6ATQYC5jXqVLBjCH8R6AUAuKjta6LsFcG4gUND1";
        assert_eq!(
            app_id(did),
            u128::from_str_radix("48b1da600caab75fa73e4fe334bf1482", 16).unwrap()
        );
        assert!(!NameId::is_reserved_id(app_id(did)));
    }

    #[test]
    fn a_channel_belongs_only_to_the_sub_that_derives_it() {
        let (data, ctrl) = channel_ids("alice", "org", "ns", "rfq").unwrap();
        let channel = ProtoName::from_strings(["org", "ns", "rfq"]).with_id(data);
        let control = ProtoName::from_strings(["org", "ns", "rfq"]).with_id(ctrl);
        assert!(is_channel_of("alice", &channel, &control));
        assert!(!is_channel_of("mallory", &channel, &control));
        assert!(!is_channel_of("alice", &control, &channel));

        // Same ids under another readable name.
        let other = ProtoName::from_strings(["org", "ns", "other"]).with_id(data);
        assert!(!is_channel_of("alice", &other, &control));
    }
}
