// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

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
}
