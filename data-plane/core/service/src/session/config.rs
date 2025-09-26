// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use slim_datapath::messages::Name;

use crate::session::multicast::MulticastConfiguration;
use crate::session::point_to_point::PointToPointConfiguration;

#[derive(Clone, PartialEq, Debug)]
pub enum SessionConfig {
    PointToPoint(PointToPointConfiguration),
    Multicast(MulticastConfiguration),
}

impl std::fmt::Display for SessionConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionConfig::PointToPoint(ff) => write!(f, "{}", ff),
            SessionConfig::Multicast(s) => write!(f, "{}", s),
        }
    }
}

impl SessionConfig {
    pub fn metadata(&self) -> HashMap<String, String> {
        match self {
            SessionConfig::PointToPoint(c) => c.metadata.clone(),
            SessionConfig::Multicast(c) => c.metadata.clone(),
        }
    }

    pub fn destination_name(&self) -> Option<Name> {
        match self {
            SessionConfig::PointToPoint(c) => c.unicast_name.as_ref().cloned(),
            SessionConfig::Multicast(c) => Some(c.channel_name.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn make_metadata() -> HashMap<String, String> {
        let mut m = HashMap::new();
        m.insert("k1".to_string(), "v1".to_string());
        m.insert("k2".to_string(), "v2".to_string());
        m
    }

    #[test]
    fn test_point_to_point_metadata_and_destination() {
        let meta = make_metadata();
        let name = Name::from_strings(["org", "ns", "endpoint"]).with_id(42);

        let cfg = PointToPointConfiguration::new(
            Some(Duration::from_millis(500)),
            Some(3),
            false,
            Some(name.clone()),
            meta.clone(),
        );
        let sc = SessionConfig::PointToPoint(cfg.clone());

        // metadata clone matches
        assert_eq!(sc.metadata(), meta);

        // destination name is the unicast name
        assert_eq!(sc.destination_name(), Some(name.clone()));

        // Display delegates to inner config
        assert_eq!(format!("{}", sc), format!("{}", cfg));
    }

    #[test]
    fn test_point_to_point_destination_none_when_unset() {
        let cfg = PointToPointConfiguration::new(None, None, false, None, HashMap::new());
        let sc = SessionConfig::PointToPoint(cfg);
        assert!(sc.destination_name().is_none());
    }

    #[test]
    fn test_multicast_metadata_and_destination() {
        let meta = make_metadata();
        let channel = Name::from_strings(["org", "ns", "channel"]).with_id(7);

        let cfg = MulticastConfiguration::new(
            channel.clone(),
            Some(10),
            Some(Duration::from_millis(1000)),
            false,
            meta.clone(),
        );
        let sc = SessionConfig::Multicast(cfg.clone());

        assert_eq!(sc.metadata(), meta);
        assert_eq!(sc.destination_name(), Some(channel.clone()));
        assert_eq!(format!("{}", sc), format!("{}", cfg));
    }
}
