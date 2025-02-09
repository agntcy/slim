// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use crate::pool::Pool;
use crate::{SubscriptionTable, SubscriptionTableError};
use agp_gw_messages::encoder::{Agent, AgentClass};
use parking_lot::{lock_api::RwLockWriteGuard, RawRwLock, RwLock};
use rand::Rng;
use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use tracing::{debug, error, warn};

#[derive(Debug, Default, Clone)]
struct ConnId {
    conn_id: u64,   // connection id
    counter: usize, // number of references
}

impl ConnId {
    fn new(conn_id: u64) -> Self {
        ConnId {
            conn_id,
            counter: 1,
        }
    }
}

#[derive(Debug)]
struct ClassState {
    // map agent_id -> vec<connection>
    // the number of connections per agent id should be small
    ids: HashMap<u64, Vec<u64>>,
    // map from connection id to position in connections
    // to be used in the insertion/remove
    connections_index: HashMap<u64, usize>,
    // list of all connections for this class
    // to be used in the match
    connections: Pool<ConnId>,
}

impl Default for ClassState {
    fn default() -> Self {
        ClassState {
            ids: HashMap::new(),
            connections_index: HashMap::new(),
            connections: Pool::with_capacity(8),
        }
    }
}

impl ClassState {
    fn new(agent_id: u64, conn: u64) -> Self {
        let mut class_state = ClassState::default();
        let conn_id = ConnId::new(conn);
        let pos = class_state.connections.insert(conn_id);
        if pos.is_none() {
            panic!("error adding a connection in the pool");
        }
        class_state.connections_index.insert(conn, pos.unwrap());
        let v = vec![conn];
        class_state.ids.insert(agent_id, v);
        class_state
    }

    fn insert(&mut self, agent_id: u64, conn: u64) {
        self.insert_connection(conn);
        match self.ids.get_mut(&agent_id) {
            None => {
                // the agent id does not exists
                let v = vec![conn];
                self.ids.insert(agent_id, v);
            }
            Some(v) => {
                v.push(conn);
            }
        }
    }

    fn insert_connection(&mut self, conn: u64) {
        match self.connections_index.get(&conn) {
            None => {
                // add new connections
                let conn_id = ConnId::new(conn);
                let pos = self.connections.insert(conn_id);
                if pos.is_none() {
                    panic!("error adding a connection in the pool");
                }
                self.connections_index.insert(conn, pos.unwrap());
            }
            Some(pos) => {
                // the connection is in the list no need to add it again
                // we just increase the counter
                let conn_id = self.connections.get_mut(*pos);
                if conn_id.is_none() {
                    panic!("error retrieving the connection from the pool");
                }
                conn_id.unwrap().counter += 1;
            }
        }
    }

    fn remove(&mut self, agent_id: u64, conn: u64) -> Result<(), SubscriptionTableError> {
        match self.ids.get_mut(&agent_id) {
            None => {
                warn!("agent id {} not found", agent_id);
                Err(SubscriptionTableError::AgentIdNotFound)
            }
            Some(v) => {
                let mut i = 0;
                let mut found = false;
                for c in v.iter() {
                    if *c == conn {
                        found = true;
                        // connection found, remove it
                        let conn_index_opt = self.connections_index.get(&conn);
                        if conn_index_opt.is_none() {
                            error!("cannot find the index for connection {}", conn);
                            return Err(SubscriptionTableError::ConnectionIdNotFound);
                        }
                        let conn_index = conn_index_opt.unwrap();
                        let conn_id_opt = self.connections.get_mut(*conn_index);
                        if conn_id_opt.is_none() {
                            error!("cannot find the connection {} in the pool", conn);
                            return Err(SubscriptionTableError::ConnectionIdNotFound);
                        }
                        let conn_id = conn_id_opt.unwrap();
                        if conn_id.counter == 1 {
                            // remove connection
                            self.connections.remove(*conn_index);
                            self.connections_index.remove(&conn);
                        } else {
                            conn_id.counter -= 1;
                        }
                        // all done
                        break;
                    }
                    i += 1;
                }
                if found {
                    // remove entry in v at position i
                    v.swap_remove(i);
                    // if v is empty after the removal we need to remove the agent from the table
                    if v.is_empty() {
                        self.ids.remove(&agent_id);
                    }
                } else {
                    warn!("connection id {} not found for agent id {}", conn, agent_id);
                    return Err(SubscriptionTableError::ConnectionIdNotFound);
                }
                Ok(())
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct SubscriptionTableImpl {
    // subscriptions table
    // agent_class -> class state
    // if a subscription comes for a specific agent_id, it is added
    // to that specific agent_id, otherwise the connection is added
    // to the DEFAULT_AGENT_ID
    table: RwLock<HashMap<AgentClass, ClassState>>,
    // connections tables
    // conn_index -> set(agent)
    connections: RwLock<HashMap<u64, HashSet<Agent>>>,
    // local connections (only one is supporterd at the moment)
    local_connections: RwLock<Vec<u64>>,
}

impl Display for SubscriptionTableImpl {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // print main table
        let table = self.table.read();
        writeln!(f, "Subscription Table")?;
        for (k, v) in table.iter() {
            writeln!(f, "Class: {:?}", k)?;
            writeln!(f, "  Agents:")?;
            for (id, conn) in v.ids.iter() {
                writeln!(f, "    Agent id: {}", id)?;
                for c in conn {
                    writeln!(f, "      Connection: {}", c)?;
                }
            }
        }

        Ok(())
    }
}

fn add_subscription_to_sub_table(
    agent: &Agent,
    conn: u64,
    mut table: RwLockWriteGuard<'_, RawRwLock, HashMap<AgentClass, ClassState>>,
) {
    match table.get_mut(&agent.agent_class) {
        None => {
            debug!(
                "subscription table: add first subscription for class {:?}, agent_id {:?} on connection {}",
                agent.agent_class, agent.agent_id, conn,
            );
            // the subscription does not exists, init
            // create and init class state
            let class_state = ClassState::new(agent.agent_id, conn);

            // insert the map in the table
            table.insert(agent.agent_class.clone(), class_state);
        }
        Some(state) => {
            state.insert(agent.agent_id, conn);
        }
    }
}

fn add_subscription_to_connection(
    agent: &Agent,
    conn_index: u64,
    mut map: RwLockWriteGuard<'_, RawRwLock, HashMap<u64, HashSet<Agent>>>,
) -> Result<(), SubscriptionTableError> {
    let set = map.get_mut(&conn_index);
    match set {
        None => {
            debug!(
                "add first subscription for class {:?}, agent_id {} on connection {}",
                agent.agent_class, agent.agent_id, conn_index,
            );
            let mut set = HashSet::new();
            set.insert(agent.clone());
            map.insert(conn_index, set);
        }
        Some(s) => {
            if !s.insert(agent.clone()) {
                warn!(
                    "subscription for class {:?}, agent_id {} already exists for connection {}",
                    agent.agent_class, agent.agent_id, conn_index,
                );
                return Err(SubscriptionTableError::ConnectionExists);
            }
        }
    }
    debug!(
        "subscription for class {:?}, agent_id {} successfully added on connection {}",
        agent.agent_class, agent.agent_id, conn_index,
    );
    Ok(())
}

fn remove_subscription_from_sub_table(
    agent: &Agent,
    conn_index: u64,
    mut table: RwLockWriteGuard<'_, RawRwLock, HashMap<AgentClass, ClassState>>,
) -> Result<(), SubscriptionTableError> {
    match table.get_mut(&agent.agent_class) {
        None => {
            debug!("subscription not found{:?}", agent.agent_class);
            Err(SubscriptionTableError::SubscriptionNotFound)
        }
        Some(state) => {
            state.remove(agent.agent_id, conn_index)?;
            // we may need to remove the state
            if state.ids.is_empty() {
                table.remove(&agent.agent_class);
            }
            Ok(())
        }
    }
}

fn remove_subscription_from_connection(
    agent: &Agent,
    conn_index: u64,
    mut map: RwLockWriteGuard<'_, RawRwLock, HashMap<u64, HashSet<Agent>>>,
) -> Result<(), SubscriptionTableError> {
    let set = map.get_mut(&conn_index);
    match set {
        None => {
            warn!("connection id {:?} not found", conn_index);
            return Err(SubscriptionTableError::ConnectionIdNotFound);
        }
        Some(s) => {
            if !s.remove(agent) {
                warn!(
                    "subscription for class {:?}, agent_id {} not found on connection {}",
                    agent.agent_class, agent.agent_id, conn_index,
                );
                return Err(SubscriptionTableError::SubscriptionNotFound);
            }
            if s.is_empty() {
                map.remove(&conn_index);
            }
        }
    }
    debug!(
        "subscription for class {:?}, agent_id {} successfully removed on connection {}",
        agent.agent_class, agent.agent_id, conn_index,
    );
    Ok(())
}

impl SubscriptionTableImpl {
    pub fn add_local_connection(&self, conn: u64) {
        let mut lock = self.local_connections.write();
        lock.push(conn);
    }

    pub fn get_local_connection(&self) -> Option<u64> {
        let local_conn = self.local_connections.read();
        if !local_conn.is_empty() {
            return Some(local_conn[0]);
        }
        None
    }
}

impl SubscriptionTable for SubscriptionTableImpl {
    fn add_subscription(
        &self,
        class: AgentClass,
        agent_id: Option<u64>,
        conn: u64,
    ) -> Result<(), SubscriptionTableError> {
        let agent = Agent {
            agent_class: class,
            agent_id: agent_id.unwrap_or(Self::DEFAULT_AGENT_ID),
        };
        {
            let conn_table = self.connections.read();
            match conn_table.get(&conn) {
                None => {}
                Some(set) => {
                    if set.contains(&agent) {
                        warn!(
                            "sub scription {:?} on connection {:?} already exists",
                            agent, conn
                        );
                        return Err(SubscriptionTableError::SubscriptionExists);
                    }
                }
            }
        }
        {
            let table = self.table.write();
            add_subscription_to_sub_table(&agent, conn, table);
        }
        {
            let conn_table = self.connections.write();
            add_subscription_to_connection(&agent, conn, conn_table)?;
        }
        Ok(())
    }

    fn remove_subscription(
        &self,
        class: AgentClass,
        agent_id: Option<u64>,
        conn: u64,
    ) -> Result<(), SubscriptionTableError> {
        let agent = Agent {
            agent_class: class,
            agent_id: agent_id.unwrap_or(Self::DEFAULT_AGENT_ID),
        };
        {
            let table = self.table.write();
            remove_subscription_from_sub_table(&agent, conn, table)?
        }
        {
            let conn_table = self.connections.write();
            remove_subscription_from_connection(&agent, conn, conn_table)?
        }
        Ok(())
    }

    fn remove_connection(&self, conn: u64) -> Result<(), SubscriptionTableError> {
        {
            let conn_map = self.connections.read();
            let set = conn_map.get(&conn);
            if set.is_none() {
                return Err(SubscriptionTableError::ConnectionIdNotFound);
            }
            for agent in set.unwrap() {
                let table = self.table.write();
                remove_subscription_from_sub_table(agent, conn, table)?;
            }
        }
        {
            let mut conn_map = self.connections.write();
            conn_map.remove(&conn); // here the connection must exists.
        }
        Ok(())
    }

    fn match_one(
        &self,
        class: AgentClass,
        agent_id: Option<u64>,
        incoming_conn: u64,
    ) -> Result<u64, SubscriptionTableError> {
        let table = self.table.read();
        let class = table.get(&class);
        match class {
            None => {
                debug!("match not found for class {:?}", class);
                Err(SubscriptionTableError::MatchNotFound)
            }
            Some(class_state) => {
                match agent_id {
                    None => {
                        // we can get a random value in class_state.
                        if class_state.connections.is_empty() {
                            // should never happen
                            error!("the connection pool is empty");
                            return Err(SubscriptionTableError::MatchNotFound);
                        }
                        if class_state.connections.len() == 1 {
                            if class_state.connections_index.contains_key(&incoming_conn) {
                                error!("the only available connection cannot be used");
                                return Err(SubscriptionTableError::MatchNotFound);
                            } else {
                                let val = class_state.connections_index.iter().next().unwrap();
                                return Ok(*val.0);
                            }
                        }

                        // we need to iterate and find a value starting from a random point in the pool
                        let mut rng = rand::rng();
                        let index = rng.random_range(0..class_state.connections.max_set() + 1);
                        let mut stop = false;
                        let mut i = index;
                        while !stop {
                            let opt = class_state.connections.get(i);
                            if opt.is_some() {
                                let out = opt.unwrap().conn_id;
                                if out != incoming_conn {
                                    return Ok(out);
                                }
                            }
                            i = (i + 1) % (class_state.connections.max_set() + 1);
                            if i == index {
                                stop = true;
                            }
                        }
                        error!("no output connection available");
                        Err(SubscriptionTableError::MatchNotFound)
                    }
                    Some(id) => {
                        // match the id. if it exists get a random value in the set
                        let val = class_state.ids.get(&id);
                        match val {
                            None => {
                                debug!("match not found for class {:?} and id {:?}", class, id);
                                Err(SubscriptionTableError::MatchNotFound)
                            }
                            Some(vec) => {
                                if vec.is_empty() {
                                    // should never happen
                                    error!("the connection list is empty");
                                    return Err(SubscriptionTableError::MatchNotFound);
                                }
                                if vec.len() == 1 {
                                    if vec[0] == incoming_conn {
                                        error!("the only available connection cannot be used");
                                        return Err(SubscriptionTableError::MatchNotFound);
                                    } else {
                                        return Ok(vec[0]);
                                    }
                                }
                                // we need to iterate an find a value starting from a random point in the vec
                                let mut rng = rand::rng();
                                let index = rng.random_range(0..vec.len());
                                let mut stop = false;
                                let mut i = index;
                                while !stop {
                                    if vec[i] != incoming_conn {
                                        return Ok(vec[index]);
                                    }
                                    i = (i + 1) % vec.len();
                                    if i == index {
                                        stop = true;
                                    }
                                }
                                error!("no output connection available");
                                Err(SubscriptionTableError::MatchNotFound)
                            }
                        }
                    }
                }
            }
        }
    }

    fn match_all(
        &self,
        class: AgentClass,
        agent_id: Option<u64>,
        incoming_conn: u64,
    ) -> Result<Vec<u64>, SubscriptionTableError> {
        let table = self.table.read();
        let class = table.get(&class);
        match class {
            None => {
                debug!("match not found for class {:?}", class);
                Err(SubscriptionTableError::MatchNotFound)
            }
            Some(state) => {
                let id = agent_id.unwrap_or(Self::DEFAULT_AGENT_ID);
                let vec = state.ids.get(&id);
                match vec {
                    None => {
                        debug!("match not found for class {:?} and id {:?}", class, id);
                        Err(SubscriptionTableError::MatchNotFound)
                    }
                    Some(conn_vec) => {
                        // remove incoming conn from the vector and return
                        let mut out = Vec::new();
                        for conn in conn_vec {
                            if *conn != incoming_conn {
                                out.push(*conn);
                            }
                        }
                        if out.is_empty() {
                            debug!("match not found for class {:?} and id {:?}", class, id);
                            return Err(SubscriptionTableError::MatchNotFound);
                        }
                        Ok(out)
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use agp_gw_messages::encoder::encode_agent_class;
    use tracing_test::traced_test;

    #[test]
    #[traced_test]
    fn test_table() {
        let agent_class1 = encode_agent_class("Cisco", "Default", "class_ONE");
        let agent_class2 = encode_agent_class("Cisco", "Default", "class_TWO");
        let agent_class3 = encode_agent_class("Cisco", "Default", "class_THREE");

        let t = SubscriptionTableImpl::default();

        assert_eq!(t.add_subscription(agent_class1.clone(), None, 1), Ok(()));
        assert_eq!(t.add_subscription(agent_class1.clone(), None, 2), Ok(()));
        assert_eq!(t.add_subscription(agent_class1.clone(), Some(1), 2), Ok(()));
        assert_eq!(t.add_subscription(agent_class2.clone(), Some(2), 3), Ok(()));

        // returns 2 matches on connection 1 and 2
        let out = t.match_all(agent_class1.clone(), None, 100).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&1));
        assert!(out.contains(&2));

        // return 1 match on connection 2
        let out = t.match_all(agent_class1.clone(), None, 1).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&2));

        assert_eq!(t.remove_subscription(agent_class1.clone(), None, 2), Ok(()));

        // return 1 match on connection 1
        let out = t.match_all(agent_class1.clone(), None, 100).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        // return no match
        assert_eq!(
            t.match_all(agent_class1.clone(), None, 1),
            Err(SubscriptionTableError::MatchNotFound)
        );

        // add subscription again
        assert_eq!(t.add_subscription(agent_class1.clone(), None, 2), Ok(()));

        // returns 2 matches on connection 1 and 2
        let out = t.match_all(agent_class1.clone(), None, 100).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&1));
        assert!(out.contains(&2));

        // run multiple times for randomenes
        for _ in 0..20 {
            let out = t.match_one(agent_class1.clone(), None, 100).unwrap();
            if out != 1 && out != 2 {
                // the output must be 1 or 2
                assert!(false);
            }
        }

        // return connection 2
        let out = t.match_one(agent_class1.clone(), Some(1), 100).unwrap();
        assert_eq!(out, 2);

        // return connection 3
        let out = t.match_one(agent_class2.clone(), Some(2), 100).unwrap();
        assert_eq!(out, 3);

        assert_eq!(t.remove_connection(2), Ok(()));

        // returns 1 match on connection 1
        let out = t.match_all(agent_class1.clone(), None, 100).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        assert_eq!(t.add_subscription(agent_class2.clone(), Some(2), 4), Ok(()));
        // run multiple times for randomenes
        for _ in 0..20 {
            let out = t.match_one(agent_class2.clone(), Some(2), 100).unwrap();
            if out != 3 && out != 4 {
                // the output must be 2 or 4
                assert!(false);
            }
        }

        assert_eq!(
            t.remove_subscription(agent_class2.clone(), Some(2), 4),
            Ok(())
        );

        // test errors
        assert_eq!(
            t.remove_connection(2),
            Err(SubscriptionTableError::ConnectionIdNotFound)
        );
        assert_eq!(
            t.match_one(agent_class1.clone(), Some(1), 100),
            Err(SubscriptionTableError::MatchNotFound)
        );
        assert_eq!(
            t.add_subscription(agent_class2.clone(), Some(2), 3),
            Err(SubscriptionTableError::SubscriptionExists)
        );
        assert_eq!(
            t.remove_subscription(agent_class3.clone(), None, 2),
            Err(SubscriptionTableError::SubscriptionNotFound)
        );
        assert_eq!(
            t.remove_subscription(agent_class2.clone(), None, 2),
            Err(SubscriptionTableError::AgentIdNotFound)
        );
    }
}
