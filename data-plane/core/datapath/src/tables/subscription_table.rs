// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};

use parking_lot::{RawRwLock, RwLock, lock_api::RwLockWriteGuard};
use rand::Rng;
use tracing::{debug, error, warn};

use super::pool::Pool;
use super::{SubscriptionTable, errors::SubscriptionTableError};
use crate::messages::Name;

#[derive(Debug, Clone)]
struct InternalName(Name);

impl Hash for InternalName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.components()[0].hash(state);
        self.0.components()[1].hash(state);
        self.0.components()[2].hash(state);
    }
}

impl PartialEq for InternalName {
    fn eq(&self, other: &Self) -> bool {
        // check only the first 3 components
        self.0.components()[0..3] == other.0.components()[0..3]
    }
}

impl Eq for InternalName {}

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
            if let Some(opt) = opt
                && opt.conn_id != except_conn
            {
                return Some(opt.conn_id);
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
        if out.is_empty() { None } else { Some(out) }
    }
}

#[derive(Debug, Default)]
struct NameState {
    // map name -> [local connection ids, remote connection ids]
    // the array contains the local connections at position 0 and the
    // remote ones at position 1
    // the number of connections per name is expected to be small
    ids: HashMap<u64, [Vec<u64>; 2]>,
    // List of all the connections that are available for this name
    // as for the ids map position 0 stores local connections and position
    // 1 store remotes ones
    connections: [Connections; 2],
}

impl NameState {
    fn new(id: u64, conn: u64, is_local: bool) -> Self {
        let mut type_state = NameState::default();
        let v = vec![conn];
        if is_local {
            type_state.connections[0].insert(conn);
            type_state.ids.insert(id, [v, vec![]]);
        } else {
            type_state.connections[1].insert(conn);
            type_state.ids.insert(id, [vec![], v]);
        }
        type_state
    }

    fn insert(&mut self, id: u64, conn: u64, is_local: bool) {
        let mut index = 0;
        if !is_local {
            index = 1;
        }
        self.connections[index].insert(conn);

        match self.ids.get_mut(&id) {
            None => {
                // the id does not exists
                let mut connections = [vec![], vec![]];
                connections[index].push(conn);
                self.ids.insert(id, connections);
            }
            Some(v) => {
                v[index].push(conn);
            }
        }
    }

    fn remove(
        &mut self,
        id: &u64,
        conn: u64,
        is_local: bool,
    ) -> Result<(), SubscriptionTableError> {
        match self.ids.get_mut(id) {
            None => {
                warn!("id {} not found", id);
                Err(SubscriptionTableError::IdNotFound)
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
                        // if both vectors are empty remove the id from the tabales
                        if connection_ids[0].is_empty() && connection_ids[1].is_empty() {
                            self.ids.remove(id);
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
        id: u64,
        incoming_conn: u64,
        get_local_connection: bool,
    ) -> Option<u64> {
        let mut index = 0;
        if !get_local_connection {
            index = 1;
        }

        if id == Name::NULL_COMPONENT {
            return self.connections[index].get_one(incoming_conn);
        }

        let val = self.ids.get(&id);
        match val {
            None => {
                // If there is only 1 connection for the name, we can still
                // try to use it
                if self.connections[index].index.len() == 1 {
                    return self.connections[index].get_one(incoming_conn);
                }

                // We cannot return any connection for this name
                debug!("cannot find out connection, name does not exists {:?}", id);
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
                    if vec[index][i] != incoming_conn {
                        return Some(vec[index][i]);
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

    fn get_all_connections(
        &self,
        id: u64,
        incoming_conn: u64,
        get_local_connection: bool,
    ) -> Option<Vec<u64>> {
        let mut index = 0;
        if !get_local_connection {
            index = 1;
        }

        if id == Name::NULL_COMPONENT {
            return self.connections[index].get_all(incoming_conn);
        }

        let val = self.ids.get(&id);
        match val {
            None => {
                debug!("cannot find out connection, id does not exists {:?}", id);
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

#[derive(Debug, Default)]
pub struct SubscriptionTableImpl {
    // subscriptions table
    // name -> name state
    // if a subscription comes for a specific id, it is added
    // to that specific id, otherwise the connection is added
    // to the Name::NULL_COMPONENT id
    table: RwLock<HashMap<InternalName, NameState>>,
    // connections tables
    // conn_index -> set(name)
    connections: RwLock<HashMap<u64, HashSet<Name>>>,
}

impl Display for SubscriptionTableImpl {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        // print main table
        let table = self.table.read();
        writeln!(f, "Subscription Table")?;
        for (k, v) in table.iter() {
            writeln!(f, "Type: {:?}", k)?;
            writeln!(f, "  Names:")?;
            for (id, conn) in v.ids.iter() {
                writeln!(f, "    Id: {}", id)?;
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
    name: Name,
    conn: u64,
    is_local: bool,
    mut table: RwLockWriteGuard<'_, RawRwLock, HashMap<InternalName, NameState>>,
) {
    let uid = name.id();
    let internal_name = InternalName(name);

    match table.get_mut(&internal_name) {
        None => {
            debug!(
                "subscription table: add first subscription for {} on connection {}",
                internal_name.0, conn
            );
            // the subscription does not exists, init
            // create and init type state
            let state = NameState::new(uid, conn, is_local);

            // insert the map in the table
            table.insert(internal_name, state);
        }
        Some(state) => {
            state.insert(uid, conn, is_local);
        }
    }
}

fn add_subscription_to_connection(
    name: Name,
    conn_index: u64,
    mut map: RwLockWriteGuard<'_, RawRwLock, HashMap<u64, HashSet<Name>>>,
) -> Result<(), SubscriptionTableError> {
    let name_str = name.to_string();

    let set = map.get_mut(&conn_index);
    match set {
        None => {
            debug!(
                "add first subscription for name {} on connection {}",
                name_str, conn_index,
            );
            let mut set = HashSet::new();
            set.insert(name);
            map.insert(conn_index, set);
        }
        Some(s) => {
            debug!(
                "add subscription for name {} on connection {}",
                name_str, conn_index,
            );

            if !s.insert(name) {
                warn!(
                    "subscription for name {} already exists for connection {}, ignore the message",
                    name_str, conn_index,
                );
                return Ok(());
            }
        }
    }
    debug!(
        "subscription for name {} successfully added on connection {}",
        name_str, conn_index,
    );
    Ok(())
}

fn remove_subscription_from_sub_table(
    name: &Name,
    conn_index: u64,
    is_local: bool,
    table: &mut RwLockWriteGuard<'_, RawRwLock, HashMap<InternalName, NameState>>,
) -> Result<(), SubscriptionTableError> {
    // Convert &Name to &InternalName. This is unsafe, but we know the types are compatible.
    let query_name = unsafe { std::mem::transmute::<&Name, &InternalName>(name) };

    match table.get_mut(query_name) {
        None => {
            debug!("subscription not found {}", name);
            Err(SubscriptionTableError::SubscriptionNotFound)
        }
        Some(state) => {
            state.remove(&name.id(), conn_index, is_local)?;
            // we may need to remove the state
            if state.ids.is_empty() {
                table.remove(query_name);
            }
            Ok(())
        }
    }
}

fn remove_subscription_from_connection(
    name: &Name,
    conn_index: u64,
    mut map: RwLockWriteGuard<'_, RawRwLock, HashMap<u64, HashSet<Name>>>,
) -> Result<(), SubscriptionTableError> {
    let set = map.get_mut(&conn_index);
    match set {
        None => {
            warn!(%conn_index, "connection not found");
            return Err(SubscriptionTableError::ConnectionIdNotFound);
        }
        Some(s) => {
            if !s.remove(name) {
                warn!(
                    "subscription for name {} not found on connection {}",
                    name, conn_index,
                );
                return Err(SubscriptionTableError::SubscriptionNotFound);
            }
            if s.is_empty() {
                map.remove(&conn_index);
            }
        }
    }
    debug!(
        "subscription for name {} successfully removed on connection {}",
        name, conn_index,
    );
    Ok(())
}

impl SubscriptionTable for SubscriptionTableImpl {
    fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(&Name, u64, &[u64], &[u64]),
    {
        let table = self.table.read();

        for (k, v) in table.iter() {
            for (id, conn) in v.ids.iter() {
                f(&k.0, *id, conn[0].as_ref(), conn[1].as_ref());
            }
        }
    }

    fn add_subscription(
        &self,
        name: Name,
        conn: u64,
        is_local: bool,
    ) -> Result<(), SubscriptionTableError> {
        {
            let conn_table = self.connections.read();
            match conn_table.get(&conn) {
                None => {}
                Some(set) => {
                    if set.contains(&name) {
                        debug!(
                            "subscription {:?} on connection {:?} already exists, ignore the message",
                            name, conn
                        );
                        return Ok(());
                    }
                }
            }
        }
        {
            let table = self.table.write();
            add_subscription_to_sub_table(name.clone(), conn, is_local, table);
        }
        {
            let conn_table = self.connections.write();
            add_subscription_to_connection(name, conn, conn_table)?;
        }
        Ok(())
    }

    fn remove_subscription(
        &self,
        name: &Name,
        conn: u64,
        is_local: bool,
    ) -> Result<(), SubscriptionTableError> {
        {
            let mut table = self.table.write();
            remove_subscription_from_sub_table(name, conn, is_local, &mut table)?;
        }
        {
            let conn_table = self.connections.write();
            remove_subscription_from_connection(name, conn, conn_table)?
        }
        Ok(())
    }

    fn remove_connection(
        &self,
        conn: u64,
        is_local: bool,
    ) -> Result<HashSet<Name>, SubscriptionTableError> {
        let removed_subscriptions = self
            .connections
            .write()
            .remove(&conn)
            .ok_or(SubscriptionTableError::ConnectionIdNotFound)?;
        let mut table = self.table.write();
        for name in &removed_subscriptions {
            debug!("remove subscription {} from connection {}", name, conn);
            remove_subscription_from_sub_table(name, conn, is_local, &mut table)?;
        }
        Ok(removed_subscriptions)
    }

    fn match_one(&self, name: &Name, incoming_conn: u64) -> Result<u64, SubscriptionTableError> {
        let table = self.table.read();

        let query_name = unsafe { std::mem::transmute::<&Name, &InternalName>(name) };

        match table.get(query_name) {
            None => {
                debug!("match not found for type {:}", name);
                Err(SubscriptionTableError::NoMatch(format!("{}", name)))
            }
            Some(state) => {
                // first try to send the message to the local connections
                // if no local connection exists or the message cannot
                // be sent try on remote ones
                let local_out = state.get_one_connection(name.id(), incoming_conn, true);
                if let Some(out) = local_out {
                    return Ok(out);
                }
                let remote_out = state.get_one_connection(name.id(), incoming_conn, false);
                if let Some(out) = remote_out {
                    return Ok(out);
                }
                error!("no output connection available");
                Err(SubscriptionTableError::NoMatch(format!("{}", name)))
            }
        }
    }

    fn match_all(
        &self,
        name: &Name,
        incoming_conn: u64,
    ) -> Result<Vec<u64>, SubscriptionTableError> {
        let table = self.table.read();

        let query_name = unsafe { std::mem::transmute::<&Name, &InternalName>(name) };

        match table.get(query_name) {
            None => {
                debug!("match not found for type {:}", name);
                Err(SubscriptionTableError::NoMatch(format!("{}", name)))
            }
            Some(state) => {
                // first try to send the message to the local connections
                // if no local connection exists or the message cannot
                // be sent try on remote ones
                let local_out = state.get_all_connections(name.id(), incoming_conn, true);
                if let Some(out) = local_out {
                    debug!("found local connections {:?}", out);
                    return Ok(out);
                }

                debug!("no local connection available, trying remote connections");
                let remote_out = state.get_all_connections(name.id(), incoming_conn, false);
                if let Some(out) = remote_out {
                    debug!("found remote connections {:?}", out);
                    return Ok(out);
                }

                error!("no connection available (local/remote)");
                Err(SubscriptionTableError::NoMatch(format!("{}", name)))
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
        let name1 = Name::from_strings(["agntcy", "default", "one"]);
        let name2 = Name::from_strings(["agntcy", "default", "two"]);
        let name3 = Name::from_strings(["agntcy", "default", "three"]);

        let name1_1 = name1.clone().with_id(1);
        let name2_2 = name2.clone().with_id(2);

        let t = SubscriptionTableImpl::default();

        assert_eq!(t.add_subscription(name1.clone(), 1, false), Ok(()));
        assert_eq!(t.add_subscription(name1.clone(), 2, false), Ok(()));
        assert_eq!(t.add_subscription(name1_1.clone(), 3, false), Ok(()));
        assert_eq!(t.add_subscription(name2_2.clone(), 3, false), Ok(()));

        // returns three matches on connection 1,2,3
        let out = t.match_all(&name1, 100).unwrap();
        assert_eq!(out.len(), 3);
        assert!(out.contains(&1));
        assert!(out.contains(&2));
        assert!(out.contains(&3));

        // return two matches on connection 2,3
        let out = t.match_all(&name1, 1).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&2));
        assert!(out.contains(&3));

        assert_eq!(t.remove_subscription(&name1, 2, false), Ok(()));

        // return two matches on connection 1,3
        let out = t.match_all(&name1, 100).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&1));
        assert!(out.contains(&3));

        assert_eq!(t.remove_subscription(&name1_1, 3, false), Ok(()));

        // return one matches on connection 1
        let out = t.match_all(&name1, 100).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        // return no match
        assert_eq!(
            t.match_all(&name1, 1),
            Err(SubscriptionTableError::NoMatch(format!("{}", name1,)))
        );

        // add subscription again
        assert_eq!(t.add_subscription(name1_1.clone(), 2, false), Ok(()));

        // returns two matches on connection 1 and 2
        let out = t.match_all(&name1, 100).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&1));
        assert!(out.contains(&2));

        // run multiple times for randomenes
        for _ in 0..20 {
            let out = t.match_one(&name1, 100).unwrap();
            if out != 1 && out != 2 {
                // the output must be 1 or 2
                panic!("the output must be 1 or 2");
            }
        }

        // return connection 2
        let out = t.match_one(&name1_1, 100).unwrap();
        assert_eq!(out, 2);

        // return connection 3
        let out = t.match_one(&name2_2, 100).unwrap();
        assert_eq!(out, 3);
        let removed_subs = t.remove_connection(2, false).unwrap();
        assert_eq!(removed_subs.len(), 1);
        assert!(removed_subs.contains(&name1_1));

        // returns one match on connection 1
        let out = t.match_all(&name1, 100).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        assert_eq!(t.add_subscription(name2_2.clone(), 4, false), Ok(()));

        // run multiple times for randomness
        for _ in 0..20 {
            let out = t.match_one(&name2_2, 100).unwrap();
            if out != 3 && out != 4 {
                // the output must be 3 or 4
                panic!("the output must be 3 or 4");
            }
        }

        for _ in 0..20 {
            let out = t.match_one(&name2_2, 4).unwrap();
            if out != 3 {
                // the output must be 3
                panic!("the output must be 3");
            }
        }

        assert_eq!(t.remove_subscription(&name2_2, 4, false), Ok(()));

        // test local vs remote
        assert_eq!(t.add_subscription(name1.clone(), 2, true), Ok(()));

        // returns one match on connection 2
        let out = t.match_all(&name1, 100).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&2));

        // returns one match on connection 2
        let out = t.match_one(&name1, 100).unwrap();
        assert_eq!(out, 2);

        // fallback on remote connection and return one match on connection 1
        let out = t.match_all(&name1, 2).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        // same here
        let out = t.match_one(&name1, 2).unwrap();
        assert_eq!(out, 1);

        // test errors
        assert_eq!(
            t.remove_connection(4, false),
            Err(SubscriptionTableError::ConnectionIdNotFound)
        );

        assert_eq!(t.match_one(&name1_1, 100), Ok(2));

        assert_eq!(
            // this generates a warning
            t.add_subscription(name2_2.clone(), 3, false),
            Ok(())
        );

        assert_eq!(
            t.remove_subscription(&name3, 2, false),
            Err(SubscriptionTableError::SubscriptionNotFound)
        );
        assert_eq!(
            t.remove_subscription(&name2, 2, false),
            Err(SubscriptionTableError::IdNotFound)
        );
    }

    #[test]
    fn test_iter() {
        let name1 = Name::from_strings(["agntcy", "default", "one"]);
        let name2 = Name::from_strings(["agntcy", "default", "two"]);

        let t = SubscriptionTableImpl::default();

        assert_eq!(t.add_subscription(name1.clone(), 1, false), Ok(()));
        assert_eq!(t.add_subscription(name1.clone(), 2, false), Ok(()));
        assert_eq!(t.add_subscription(name2.clone(), 3, true), Ok(()));

        let mut h = HashMap::new();

        t.for_each(|k, id, local, remote| {
            println!(
                "key: {}, id: {}, local: {:?}, remote: {:?}",
                k, id, local, remote
            );

            h.insert(k.clone(), (id, local.to_vec(), remote.to_vec()));
        });

        assert_eq!(h.len(), 2);
        assert_eq!(h[&name1].1, vec![] as Vec<u64>);
        assert_eq!(h[&name1].2, vec![1, 2]);

        assert_eq!(h[&name2].1, vec![3]);
        assert_eq!(h[&name2].2, vec![] as Vec<u64>);
    }
}
