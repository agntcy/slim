// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::hash::{DefaultHasher, Hash, Hasher};

use crate::api::ProtoAgent;

pub const DEFAULT_AGENT_ID: u64 = u64::MAX;

#[derive(Debug, Clone, Default)]
pub struct AgentType {
    organization: u64,
    namespace: u64,
    agent_type: u64,

    // Store the original string representation of the agent type
    // This is useful for debugging and logging purposes
    strings: Option<Box<(String, String, String)>>,
}

impl Hash for AgentType {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.organization.hash(state);
        self.namespace.hash(state);
        self.agent_type.hash(state);
    }
}

impl PartialEq for AgentType {
    fn eq(&self, other: &Self) -> bool {
        self.organization == other.organization
            && self.namespace == other.namespace
            && self.agent_type == other.agent_type
    }
}

impl Eq for AgentType {}

impl std::fmt::Display for AgentType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:x}/{:x}/{:x}",
            self.organization, self.namespace, self.agent_type
        )?;

        if let Some(strings) = &self.strings {
            write!(f, " ({}/{}/{})", strings.0, strings.1, strings.2)?;
        }

        Ok(())
    }
}

impl From<&ProtoAgent> for AgentType {
    fn from(agent: &ProtoAgent) -> Self {
        Self {
            organization: agent.organization,
            namespace: agent.namespace,
            agent_type: agent.agent_type,
            strings: None,
        }
    }
}

impl AgentType {
    pub fn from_strings(organization: &str, namespace: &str, agent_type: &str) -> Self {
        Self {
            organization: calculate_hash(organization),
            namespace: calculate_hash(namespace),
            agent_type: calculate_hash(agent_type),
            strings: Some(Box::new((
                organization.to_string(),
                namespace.to_string(),
                agent_type.to_string(),
            ))),
        }
    }

    pub fn organization(&self) -> u64 {
        self.organization
    }

    pub fn namespace(&self) -> u64 {
        self.namespace
    }

    pub fn agent_type(&self) -> u64 {
        self.agent_type
    }

    pub fn organization_string(&self) -> Option<String> {
        self.strings.as_ref().map(|s| s.0.clone())
    }

    pub fn namespace_string(&self) -> Option<String> {
        self.strings.as_ref().map(|s| s.1.clone())
    }

    pub fn agent_type_string(&self) -> Option<String> {
        self.strings.as_ref().map(|s| s.2.clone())
    }
}

#[derive(Hash, Eq, PartialEq, Debug, Clone, Default)]
pub struct Agent {
    agent_type: AgentType,
    agent_id: u64,
}

impl std::fmt::Display for Agent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{:x}/{:x}/{:x}/{:x}",
            self.agent_type.organization(),
            self.agent_type.namespace(),
            self.agent_type.agent_type(),
            self.agent_id
        )
    }
}

impl From<&ProtoAgent> for Agent {
    fn from(agent: &ProtoAgent) -> Self {
        Self {
            agent_type: AgentType::from(agent),
            agent_id: agent.agent_id.expect("agent id not found"),
        }
    }
}

impl Agent {
    /// Create a new Agent
    pub fn new(agent_type: AgentType, agent_id: u64) -> Self {
        Self {
            agent_type,
            agent_id,
        }
    }

    pub fn from_strings(
        organization: &str,
        namespace: &str,
        agent_type: &str,
        agent_id: u64,
    ) -> Self {
        Self {
            agent_type: AgentType::from_strings(organization, namespace, agent_type),
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

    pub fn agent_id(&self) -> u64 {
        self.agent_id
    }

    pub fn agent_id_option(&self) -> Option<u64> {
        if self.agent_id == DEFAULT_AGENT_ID {
            return None;
        }

        Some(self.agent_id)
    }
}

pub fn calculate_hash<T: Hash + ?Sized>(t: &T) -> u64 {
    let mut s = DefaultHasher::new();
    t.hash(&mut s);
    s.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_encoder() {
        // test encode class
        let encode1 = AgentType::from_strings("Cisco", "Default", "Agent_ONE");
        let encode2 = AgentType::from_strings("Cisco", "Default", "Agent_ONE");
        assert_eq!(encode1, encode2);
        let encode3 = AgentType::from_strings("not_Cisco", "not_Default", "not_Agent_ONE");
        assert_ne!(encode1, encode3);

        let encode4 = AgentType::from_strings("Cisco", "Cisco", "Agent_ONE");
        assert_eq!(encode4.organization(), encode4.namespace());

        // test encode agent
        let agent_type = AgentType::from_strings("Cisco", "Default", "Agent_ONE");
        let agent_id = Agent::from_strings("Cisco", "Default", "Agent_ONE", 1);
        assert_eq!(agent_type, *agent_id.agent_type());
    }
}
