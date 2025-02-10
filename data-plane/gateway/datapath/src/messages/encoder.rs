// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::hash::{DefaultHasher, Hash, Hasher};

#[derive(Hash, Eq, PartialEq, Debug, Clone, Default)]
pub struct AgentClass {
    pub organization: u64,
    pub namespace: u64,
    pub agent_class: u64,
}

// TODO(msardara): refactor this to use rust traits

#[derive(Hash, Eq, PartialEq, Debug, Clone, Default)]
pub struct Agent {
    pub agent_class: AgentClass,
    pub agent_id: u64,
}

fn calculate_hash<T: Hash + ?Sized>(t: &T) -> u64 {
    let mut s = DefaultHasher::new();
    t.hash(&mut s);
    s.finish()
}

pub fn encode_agent_class(organization: &str, namespace: &str, agent_class: &str) -> AgentClass {
    AgentClass {
        organization: calculate_hash(organization),
        namespace: calculate_hash(namespace),
        agent_class: calculate_hash(agent_class),
    }
}
pub fn encode_agent_from_string(
    organization: &str,
    namespace: &str,
    agent_class: &str,
    agent_id: u64,
) -> Agent {
    Agent {
        agent_class: encode_agent_class(organization, namespace, agent_class),
        agent_id,
    }
}

pub fn encode_agent_from_class(class: AgentClass, agent_id: u64) -> Agent {
    Agent {
        agent_class: class,
        agent_id,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_name_encoder() {
        // test encode class
        let encode1 = encode_agent_class("Cisco", "Default", "Agent_ONE");
        let encode2 = encode_agent_class("Cisco", "Default", "Agent_ONE");
        assert_eq!(encode1, encode2);
        let encode3 = encode_agent_class("not_Cisco", "not_Default", "not_Agent_ONE");
        assert_ne!(encode1, encode3);

        let encode4 = encode_agent_class("Cisco", "Cisco", "Agent_ONE");
        assert_eq!(encode4.organization, encode4.namespace);

        // test encode agent
        let class = encode_agent_class("Cisco", "Default", "Agent_ONE");
        let agent = encode_agent_from_string("Cisco", "Default", "Agent_ONE", 1);
        assert_eq!(class, agent.agent_class);
        let agent_from_class = encode_agent_from_class(class.clone(), 1);
        assert_eq!(agent, agent_from_class);
    }
}
