// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};

use parking_lot::{RawRwLock, RwLock, lock_api::RwLockWriteGuard};
use rand::Rng;
use tracing::{debug, error, warn};

use super::pool::Pool;
use super::{SubscriptionTable, errors::SubscriptionTableError};
use crate::messages::encoder::DEFAULT_AGENT_ID;
use crate::messages::{Agent, AgentType};

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
struct Connections {
    // map from connection id to the position in the connections pool
    // this is used in the insertion/remove
    index: HashMap<u64, usize>,
    // pool of all connections ids that can to be used in the match
    pool: Pool<ConnId>,
}

impl Default for Connections {
    fn default() -> Self {
        Connections {
            index: HashMap::new(),
            pool: Pool::with_capacity(2),
        }
    }
}

impl Connections {
    fn insert(&mut self, conn: u64) {
        match self.index.get(&conn) {
            None => {
                let conn_id = ConnId::new(conn);
                let pos = self.pool.insert(conn_id);
                self.index.insert(conn, pos);
            }
            Some(pos) => match self.pool.get_mut(*pos) {
                None => {
                    error!("error retrieving the connection from the pool");
                }
                Some(conn_id) => {
                    conn_id.counter += 1;
                }
            },
        }
    }

    fn remove(&mut self, conn: u64) -> Result<(), SubscriptionTableError> {
        let conn_index_opt = self.index.get(&conn);
        if conn_index_opt.is_none() {
            error!("cannot find the index for connection {}", conn);
            return Err(SubscriptionTableError::ConnectionIdNotFound);
        }
        let conn_index = conn_index_opt.unwrap();
        let conn_id_opt = self.pool.get_mut(*conn_index);
        if conn_id_opt.is_none() {
            error!("cannot find the connection {} in the pool", conn);
            return Err(SubscriptionTableError::ConnectionIdNotFound);
        }
        let conn_id = conn_id_opt.unwrap();
        if conn_id.counter == 1 {
            // remove connection
            self.pool.remove(*conn_index);
            self.index.remove(&conn);
        } else {
            conn_id.counter -= 1;
        }
        Ok(())
    }

    fn get_one(&self, except_conn: u64) -> Option<u64> {
        if self.index.len() == 1 {
            if self.index.contains_key(&except_conn) {
                debug!("the only available connection cannot be used");
                return None;
            } else {
                let val = self.index.iter().next().unwrap();
                return Some(*val.0);
            }
        }

        // we need to iterate and find a value starting from a random point in the pool
        let mut rng = rand::rng();
        let index = rng.random_range(0..self.pool.max_set() + 1);
        let mut stop = false;
        let mut i = index;
        while !stop {
            let opt = self.pool.get(i);
            if opt.is_some() {
                let out = opt.unwrap().conn_id;
                if out != except_conn {
                    return Some(out);
                }
            }
            i = (i + 1) % (self.pool.max_set() + 1);
            if i == index {
                stop = true;
            }
        }
        debug!("no output connection available");
        None
    }

    fn get_all(&self, except_conn: u64) -> Option<Vec<u64>> {
        if self.index.len() == 1 {
            if self.index.contains_key(&except_conn) {
                debug!("the only available connection cannot be used");
                return None;
            } else {
                let val = self.index.iter().next().unwrap();
                return Some(vec![*val.0]);
            }
        }
        let mut out = Vec::new();
        for val in self.index.iter() {
            if *val.0 != except_conn {
                out.push(*val.0);
            }
        }
        if out.is_empty() {
            debug!("no output connection available");
            None
        } else {
            Some(out)
        }
    }
}

#[derive(Debug, Default)]
struct AgentTypeState {
    // map agent id -> [local connection ids, remote connection ids]
    // the array contains the local connections at position 0 and the
    // remote ones at position 1
    // the number of connections per agent id is expected to be small
    ids: HashMap<u64, [Vec<u64>; 2]>,
    // List of all the connections that are available for this agent type
    // as for the ids map position 0 stores local connections and position
    // 1 store remotes ones
    connections: [Connections; 2],
}

impl AgentTypeState {
    fn new(agent_id: u64, conn: u64, is_local: bool) -> Self {
        let mut type_state = AgentTypeState::default();
        let v = vec![conn];
        if is_local {
            type_state.connections[0].insert(conn);
            type_state.ids.insert(agent_id, [v, vec![]]);
        } else {
            type_state.connections[1].insert(conn);
            type_state.ids.insert(agent_id, [vec![], v]);
        }
        type_state
    }

    fn insert(&mut self, agent_id: u64, conn: u64, is_local: bool) {
        let mut index = 0;
        if !is_local {
            index = 1;
        }
        self.connections[index].insert(conn);

        match self.ids.get_mut(&agent_id) {
            None => {
                // the agent id does not exists
                let mut connections = [vec![], vec![]];
                connections[index].push(conn);
                self.ids.insert(agent_id, connections);
            }
            Some(v) => {
                v[index].push(conn);
            }
        }
    }

    fn remove(
        &mut self,
        agent_id: &u64,
        conn: u64,
        is_local: bool,
    ) -> Result<(), SubscriptionTableError> {
        match self.ids.get_mut(agent_id) {
            None => {
                warn!("agent id {} not found", agent_id);
                Err(SubscriptionTableError::AgentIdNotFound)
            }
            Some(connection_ids) => {
                let mut index = 0;
                if !is_local {
                    index = 1;
                }
                self.connections[index].remove(conn)?;
                for (i, c) in connection_ids[index].iter().enumerate() {
                    if *c == conn {
                        connection_ids[index].swap_remove(i);
                        // if both vectors are empty remove the agent id from the tabales
                        if connection_ids[0].is_empty() && connection_ids[1].is_empty() {
                            self.ids.remove(agent_id);
                        }
                        break;
                    }
                }
                Ok(())
            }
        }
    }

    fn get_one_connection(
        &self,
        agent_id: Option<u64>,
        incoming_conn: u64,
        get_local_connection: bool,
    ) -> Option<u64> {
        let mut index = 0;
        if !get_local_connection {
            index = 1;
        }
        match agent_id {
            None => self.connections[index].get_one(incoming_conn),
            Some(id) => {
                let val = self.ids.get(&id);
                match val {
                    None => {
                        // If there is only 1 connection for the agent type, we can still
                        // try to use it
                        if self.connections[index].index.len() == 1 {
                            return self.connections[index].get_one(incoming_conn);
                        }

                        // We cannot return any connection for this agent id
                        debug!(
                            "cannot find out connection, agent id does not exists {:?}",
                            id
                        );
                        None
                    }
                    Some(vec) => {
                        if vec[index].is_empty() {
                            // no connections available
                            return None;
                        }

                        if vec[index].len() == 1 {
                            if vec[index][0] == incoming_conn {
                                // cannot return the incoming interface d
                                debug!("the only available connection cannot be used");
                                return None;
                            } else {
                                return Some(vec[index][0]);
                            }
                        }

                        // we need to iterate an find a value starting from a random point in the vec
                        let mut rng = rand::rng();
                        let pos = rng.random_range(0..vec.len());
                        let mut stop = false;
                        let mut i = pos;
                        while !stop {
                            if vec[index][pos] != incoming_conn {
                                return Some(vec[index][pos]);
                            }
                            i = (i + 1) % vec[index].len();
                            if i == pos {
                                stop = true;
                            }
                        }
                        debug!("no output connection available");
                        None
                    }
                }
            }
        }
    }

    fn get_all_connections(
        &self,
        agent_id: Option<u64>,
        incoming_conn: u64,
        get_local_connection: bool,
    ) -> Option<Vec<u64>> {
        let mut index = 0;
        if !get_local_connection {
            index = 1;
        }
        match agent_id {
            None => self.connections[index].get_all(incoming_conn),
            Some(id) => {
                let val = self.ids.get(&id);
                match val {
                    None => {
                        debug!(
                            "cannot find out connection, agent id does not exists {:?}",
                            id
                        );
                        None
                    }
                    Some(vec) => {
                        if vec[index].is_empty() {
                            // should never happen
                            return None;
                        }

                        if vec[index].len() == 1 {
                            if vec[index][0] == incoming_conn {
                                // cannot return the incoming interface d
                                debug!("the only available connection cannot be used");
                                return None;
                            } else {
                                return Some(vec[index].clone());
                            }
                        }

                        // we need to iterate over the vector and remove the incoming connection
                        let mut out = Vec::new();
                        for c in vec[index].iter() {
                            if *c != incoming_conn {
                                out.push(*c);
                            }
                        }
                        if out.is_empty() { None } else { Some(out) }
                    }
                }
            }
        }
    }
}

#[derive(Debug, Default)]
pub struct SubscriptionTableImpl {
    // subscriptions table
    // agent_type -> type state
    // if a subscription comes for a specific agent_id, it is added
    // to that specific agent_id, otherwise the connection is added
    // to the DEFAULT_AGENT_ID
    table: RwLock<HashMap<AgentType, AgentTypeState>>,
    // connections tables
    // conn_index -> set(agent)
    connections: RwLock<HashMap<u64, HashSet<Agent>>>,
}

impl Display for SubscriptionTableImpl {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // print main table
        let table = self.table.read();
        writeln!(f, "Subscription Table")?;
        for (k, v) in table.iter() {
            writeln!(f, "Type: {:?}", k)?;
            writeln!(f, "  Agents:")?;
            for (id, conn) in v.ids.iter() {
                writeln!(f, "    Agent id: {}", id)?;
                if conn[0].is_empty() {
                    writeln!(f, "       Local Connections:")?;
                    writeln!(f, "         None")?;
                } else {
                    writeln!(f, "       Local Connections:")?;
                    for c in conn[0].iter() {
                        writeln!(f, "         Connection: {}", c)?;
                    }
                }
                if conn[1].is_empty() {
                    writeln!(f, "       Remote Connections:")?;
                    writeln!(f, "         None")?;
                } else {
                    writeln!(f, "       Remote Connections:")?;
                    for c in conn[1].iter() {
                        writeln!(f, "         Connection: {}", c)?;
                    }
                }
            }
        }

        Ok(())
    }
}

fn add_subscription_to_sub_table(
    agent: &Agent,
    conn: u64,
    is_local: bool,
    mut table: RwLockWriteGuard<'_, RawRwLock, HashMap<AgentType, AgentTypeState>>,
) {
    match table.get_mut(agent.agent_type()) {
        None => {
            let uid = agent.agent_id();
            debug!(
                "subscription table: add first subscription for type {}, agent_id {} on connection {}",
                agent.agent_type(),
                uid,
                conn,
            );
            // the subscription does not exists, init
            // create and init type state
            let state = AgentTypeState::new(uid, conn, is_local);

            // insert the map in the table
            table.insert(agent.agent_type().clone(), state);
        }
        Some(state) => {
            state.insert(agent.agent_id(), conn, is_local);
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
                "add first subscription for type {}, agent_id {} on connection {}",
                agent.agent_type(),
                agent.agent_id(),
                conn_index,
            );
            let mut set = HashSet::new();
            set.insert(agent.clone());
            map.insert(conn_index, set);
        }
        Some(s) => {
            if !s.insert(agent.clone()) {
                warn!(
                    "subscription for type {}, agent_id {} already exists for connection {}, ignore the message",
                    agent.agent_type(),
                    agent.agent_id(),
                    conn_index,
                );
                return Ok(());
            }
        }
    }
    debug!(
        "subscription for type {}, agent_id {} successfully added on connection {}",
        agent.agent_type(),
        agent.agent_id(),
        conn_index,
    );
    Ok(())
}

fn remove_subscription_from_sub_table(
    agent: &Agent,
    conn_index: u64,
    is_local: bool,
    mut table: RwLockWriteGuard<'_, RawRwLock, HashMap<AgentType, AgentTypeState>>,
) -> Result<(), SubscriptionTableError> {
    match table.get_mut(agent.agent_type()) {
        None => {
            debug!("subscription not found{:?}", agent.agent_type());
            Err(SubscriptionTableError::SubscriptionNotFound)
        }
        Some(state) => {
            state.remove(&agent.agent_id(), conn_index, is_local)?;
            // we may need to remove the state
            if state.ids.is_empty() {
                table.remove(agent.agent_type());
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
                    "subscription for type {}, agent_id {} not found on connection {}",
                    agent.agent_type(),
                    agent.agent_id(),
                    conn_index,
                );
                return Err(SubscriptionTableError::SubscriptionNotFound);
            }
            if s.is_empty() {
                map.remove(&conn_index);
            }
        }
    }
    debug!(
        "subscription for type {}, agent_id {} successfully removed on connection {}",
        agent.agent_type(),
        agent.agent_id(),
        conn_index,
    );
    Ok(())
}

impl SubscriptionTable for SubscriptionTableImpl {
    fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(&AgentType, u64, &[u64], &[u64]),
    {
        let table = self.table.read();

        for (k, v) in table.iter() {
            for (id, conn) in v.ids.iter() {
                f(k, *id, conn[0].as_ref(), conn[1].as_ref());
            }
        }
    }

    fn add_subscription(
        &self,
        agent_type: AgentType,
        agent_uid: Option<u64>,
        conn: u64,
        is_local: bool,
    ) -> Result<(), SubscriptionTableError> {
        let agent = Agent::new(agent_type, agent_uid.unwrap_or(DEFAULT_AGENT_ID));
        {
            let conn_table = self.connections.read();
            match conn_table.get(&conn) {
                None => {}
                Some(set) => {
                    if set.contains(&agent) {
                        debug!(
                            "subscription {:?} on connection {:?} already exists, ignore the message",
                            agent, conn
                        );
                        return Ok(());
                    }
                }
            }
        }
        {
            let table = self.table.write();
            add_subscription_to_sub_table(&agent, conn, is_local, table);
        }
        {
            let conn_table = self.connections.write();
            add_subscription_to_connection(&agent, conn, conn_table)?;
        }
        Ok(())
    }

    fn remove_subscription(
        &self,
        agent_type: AgentType,
        agent_id: Option<u64>,
        conn: u64,
        is_local: bool,
    ) -> Result<(), SubscriptionTableError> {
        let agent = Agent::new(agent_type, agent_id.unwrap_or(DEFAULT_AGENT_ID));
        {
            let table = self.table.write();

            remove_subscription_from_sub_table(&agent, conn, is_local, table)?
        }
        {
            let conn_table = self.connections.write();
            remove_subscription_from_connection(&agent, conn, conn_table)?
        }
        Ok(())
    }

    fn remove_connection(&self, conn: u64, is_local: bool) -> Result<(), SubscriptionTableError> {
        {
            let conn_map = self.connections.read();
            let set = conn_map.get(&conn);
            if set.is_none() {
                return Err(SubscriptionTableError::ConnectionIdNotFound);
            }
            for agent in set.unwrap() {
                let table = self.table.write();
                debug!("remove subscription {} from connection {}", agent, conn);
                remove_subscription_from_sub_table(agent, conn, is_local, table)?;
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
        agent_type: AgentType,
        agent_id: Option<u64>,
        incoming_conn: u64,
    ) -> Result<u64, SubscriptionTableError> {
        let table = self.table.read();
        match table.get(&agent_type) {
            None => {
                debug!("match not found for type {:}", agent_type);
                Err(SubscriptionTableError::NoMatch(format!(
                    "{}, {:?}",
                    agent_type, agent_id
                )))
            }
            Some(state) => {
                // first try to send the message to the local connections
                // if no local connection exists or the message cannot
                // be sent try on remote ones
                let local_out = state.get_one_connection(agent_id, incoming_conn, true);
                if let Some(out) = local_out {
                    return Ok(out);
                }
                let remote_out = state.get_one_connection(agent_id, incoming_conn, false);
                if let Some(out) = remote_out {
                    return Ok(out);
                }
                error!("no output connection available");
                Err(SubscriptionTableError::NoMatch(format!(
                    "{}, {:?}",
                    agent_type, agent_id
                )))
            }
        }
    }

    fn match_all(
        &self,
        agent_type: AgentType,
        agent_id: Option<u64>,
        incoming_conn: u64,
    ) -> Result<Vec<u64>, SubscriptionTableError> {
        let table = self.table.read();
        match table.get(&agent_type) {
            None => {
                debug!("match not found for type {:}", agent_type);
                Err(SubscriptionTableError::NoMatch(format!(
                    "{}, {:?}",
                    agent_type, agent_id
                )))
            }
            Some(state) => {
                // first try to send the message to the local connections
                // if no local connection exists or the message cannot
                // be sent try on remote ones
                let local_out = state.get_all_connections(agent_id, incoming_conn, true);
                if let Some(out) = local_out {
                    return Ok(out);
                }
                let remote_out = state.get_all_connections(agent_id, incoming_conn, false);
                if let Some(out) = remote_out {
                    return Ok(out);
                }
                error!("no output connection available");
                Err(SubscriptionTableError::NoMatch(format!(
                    "{}, {:?}",
                    agent_type, agent_id
                )))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tracing_test::traced_test;

    #[test]
    #[traced_test]
    fn test_table() {
        let agent_type1 = AgentType::from_strings("Cisco", "Default", "type_ONE");
        let agent_type2 = AgentType::from_strings("Cisco", "Default", "type_TWO");
        let agent_type3 = AgentType::from_strings("Cisco", "Default", "type_THREE");

        let t = SubscriptionTableImpl::default();

        assert_eq!(
            t.add_subscription(agent_type1.clone(), None, 1, false),
            Ok(())
        );
        assert_eq!(
            t.add_subscription(agent_type1.clone(), None, 2, false),
            Ok(())
        );
        assert_eq!(
            t.add_subscription(agent_type1.clone(), Some(1), 3, false),
            Ok(())
        );
        assert_eq!(
            t.add_subscription(agent_type2.clone(), Some(2), 3, false),
            Ok(())
        );

        // returns three matches on connection 1,2,3
        let out = t.match_all(agent_type1.clone(), None, 100).unwrap();
        assert_eq!(out.len(), 3);
        assert!(out.contains(&1));
        assert!(out.contains(&2));
        assert!(out.contains(&3));

        // return two matches on connection 2,3
        let out = t.match_all(agent_type1.clone(), None, 1).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&2));
        assert!(out.contains(&3));

        assert_eq!(
            t.remove_subscription(agent_type1.clone(), None, 2, false),
            Ok(())
        );

        // return two matches on connection 1,3
        let out = t.match_all(agent_type1.clone(), None, 100).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&1));
        assert!(out.contains(&3));

        assert_eq!(
            t.remove_subscription(agent_type1.clone(), Some(1), 3, false),
            Ok(())
        );

        // return one matches on connection 1
        let out = t.match_all(agent_type1.clone(), None, 100).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        // return no match
        assert_eq!(
            t.match_all(agent_type1.clone(), None, 1),
            Err(SubscriptionTableError::NoMatch(format!(
                "{}, {:?}",
                agent_type1,
                Option::<u64>::None
            )))
        );

        // add subscription again
        assert_eq!(
            t.add_subscription(agent_type1.clone(), Some(1), 2, false),
            Ok(())
        );

        // returns two matches on connection 1 and 2
        let out = t.match_all(agent_type1.clone(), None, 100).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&1));
        assert!(out.contains(&2));

        // run multiple times for randomenes
        for _ in 0..20 {
            let out = t.match_one(agent_type1.clone(), None, 100).unwrap();
            if out != 1 && out != 2 {
                // the output must be 1 or 2
                panic!("the output must be 1 or 2");
            }
        }

        // return connection 2
        let out = t.match_one(agent_type1.clone(), Some(1), 100).unwrap();
        assert_eq!(out, 2);

        // return connection 3
        let out = t.match_one(agent_type2.clone(), Some(2), 100).unwrap();
        assert_eq!(out, 3);

        assert_eq!(t.remove_connection(2, false), Ok(()));

        // returns one match on connection 1
        let out = t.match_all(agent_type1.clone(), None, 100).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        assert_eq!(
            t.add_subscription(agent_type2.clone(), Some(2), 4, false),
            Ok(())
        );

        // run multiple times for randomness
        for _ in 0..20 {
            let out = t.match_one(agent_type2.clone(), Some(2), 100).unwrap();
            if out != 3 && out != 4 {
                // the output must be 2 or 4
                panic!("the output must be 2 or 4");
            }
        }

        assert_eq!(
            t.remove_subscription(agent_type2.clone(), Some(2), 4, false),
            Ok(())
        );

        // test local vs remote
        assert_eq!(
            t.add_subscription(agent_type1.clone(), None, 2, true),
            Ok(())
        );

        // returns one match on connection 2
        let out = t.match_all(agent_type1.clone(), None, 100).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&2));

        // returns one match on connection 2
        let out = t.match_one(agent_type1.clone(), None, 100).unwrap();
        assert_eq!(out, 2);

        // fallback on remote connection and return one match on connection 1
        let out = t.match_all(agent_type1.clone(), None, 2).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        // same here
        let out = t.match_one(agent_type1.clone(), None, 2).unwrap();
        assert_eq!(out, 1);

        // test errors
        assert_eq!(
            t.remove_connection(4, false),
            Err(SubscriptionTableError::ConnectionIdNotFound)
        );
        assert_eq!(t.match_one(agent_type1.clone(), Some(1), 100), Ok(2),);
        assert_eq!(
            // this generates a warning
            t.add_subscription(agent_type2.clone(), Some(2), 3, false),
            Ok(())
        );
        assert_eq!(
            t.remove_subscription(agent_type3.clone(), None, 2, false),
            Err(SubscriptionTableError::SubscriptionNotFound)
        );
        assert_eq!(
            t.remove_subscription(agent_type2.clone(), None, 2, false),
            Err(SubscriptionTableError::AgentIdNotFound)
        );
    }

    #[test]
    fn test_iter() {
        let agent_type1 = AgentType::from_strings("Org", "Default", "type_ONE");
        let agent_type2 = AgentType::from_strings("Org", "Default", "type_TWO");

        let t = SubscriptionTableImpl::default();

        assert_eq!(
            t.add_subscription(agent_type1.clone(), None, 1, false),
            Ok(())
        );
        assert_eq!(
            t.add_subscription(agent_type1.clone(), None, 2, false),
            Ok(())
        );
        assert_eq!(
            t.add_subscription(agent_type2.clone(), None, 3, true),
            Ok(())
        );

        let mut h = HashMap::new();

        t.for_each(|k, id, local, remote| {
            println!(
                "key: {}, id: {}, local: {:?}, remote: {:?}",
                k, id, local, remote
            );

            h.insert(k.clone(), (id, local.to_vec(), remote.to_vec()));
        });

        assert_eq!(h.len(), 2);
        assert_eq!(h[&agent_type1].0, DEFAULT_AGENT_ID);
        assert_eq!(h[&agent_type1].1, vec![]);
        assert_eq!(h[&agent_type1].2, vec![1, 2]);

        assert_eq!(h[&agent_type2].0, DEFAULT_AGENT_ID);
        assert_eq!(h[&agent_type2].1, vec![3]);
        assert_eq!(h[&agent_type2].2, vec![]);
    }
}
