// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::hash::{DefaultHasher, Hash, Hasher};

pub const DEFAULT_AGENT_ID: u64 = 0;

#[derive(Hash, Eq, PartialEq, Debug, Clone, Default)]
pub struct AgentType {
    organization: u64,
    namespace: u64,
    agent_type: u64,
}

impl AgentType {
    /// Create a new AgentType
    pub fn new(organization: u64, namespace: u64, agent_type: u64) -> Self {
        Self {
            organization,
            namespace,
            agent_type,
        }
    }

    pub fn with_organization(self, organization: u64) -> Self {
        Self {
            organization,
            ..self
        }
    }

    pub fn with_namespace(self, namespace: u64) -> Self {
        Self { namespace, ..self }
    }

    pub fn with_agent_type(self, agent_type: u64) -> Self {
        Self { agent_type, ..self }
    }

    pub fn organization(&self) -> &u64 {
        &self.organization
    }

    pub fn namespace(&self) -> &u64 {
        &self.namespace
    }

    pub fn agent_type(&self) -> &u64 {
        &self.agent_type
    }
}

#[derive(Hash, Eq, PartialEq, Debug, Clone, Default)]
pub struct Agent {
    agent_type: AgentType,
    agent_id: u64,
}

impl Agent {
    /// Create a new Agent
    pub fn new(agent_type: AgentType, agent_id: u64) -> Self {
        Self {
            agent_type,
            agent_id,
        }
    }

    pub fn with_agent_id(self, agent_id: u64) -> Self {
        Self { agent_id, ..self }
    }

    pub fn with_agent_type(self, agent_type: AgentType) -> Self {
        Self { agent_type, ..self }
    }

    pub fn agent_type(&self) -> &AgentType {
        &self.agent_type
    }

    pub fn agent_id(&self) -> &u64 {
        &self.agent_id
    }

    pub fn agent_id_option(&self) -> Option<u64> {
        if self.agent_id == DEFAULT_AGENT_ID {
            return None;
        }

        Some(self.agent_id)
    }
}

fn calculate_hash<T: Hash + ?Sized>(t: &T) -> u64 {
    let mut s = DefaultHasher::new();
    t.hash(&mut s);
    s.finish()
}

pub fn encode_agent_type(organization: &str, namespace: &str, agent_type: &str) -> AgentType {
    AgentType::new(
        calculate_hash(organization),
        calculate_hash(namespace),
        calculate_hash(agent_type),
    )
}

pub fn encode_agent(
    organization: &str,
    namespace: &str,
    agent_type: &str,
    agent_uid: u64,
) -> Agent {
    Agent::new(
        encode_agent_type(organization, namespace, agent_type),
        agent_uid,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_encoder() {
        // test encode class
        let encode1 = encode_agent_type("Cisco", "Default", "Agent_ONE");
        let encode2 = encode_agent_type("Cisco", "Default", "Agent_ONE");
        assert_eq!(encode1, encode2);
        let encode3 = encode_agent_type("not_Cisco", "not_Default", "not_Agent_ONE");
        assert_ne!(encode1, encode3);

        let encode4 = encode_agent_type("Cisco", "Cisco", "Agent_ONE");
        assert_eq!(encode4.organization(), encode4.namespace());

        // test encode agent
        let agent_type = encode_agent_type("Cisco", "Default", "Agent_ONE");
        let agent_id = encode_agent("Cisco", "Default", "Agent_ONE", 1);
        assert_eq!(agent_type, *agent_id.agent_type());
    }
}
