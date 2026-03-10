// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::{HashMap, HashSet};
use std::fmt::{Display, Formatter};
use std::hash::{Hash, Hasher};

use parking_lot::{RawRwLock, RwLock, lock_api::RwLockWriteGuard};
use rand::Rng;
use tracing::{debug, error, warn};

use super::SubscriptionTable;
use super::pool::Pool;
use crate::errors::DataPathError;
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

#[derive(Debug, Default)]
struct SubscriptionRefs {
    // map from connection id to reference counter
    refs: HashMap<u64, usize>,
}

impl SubscriptionRefs {
    fn new(conn: u64) -> Self {
        let refs = HashMap::from([(conn, 1)]);
        SubscriptionRefs { refs }
    }

    fn insert(&mut self, conn: u64) {
        *self.refs.entry(conn).or_insert(0) += 1;
    }

    fn remove(&mut self, conn: u64) -> Result<bool, DataPathError> {
        match self.refs.get_mut(&conn) {
            None => {
                debug!(%conn, "connection not found in refs");
                Err(DataPathError::ConnectionIdNotFound(conn))
            }
            Some(count) => {
                if *count == 1 {
                    self.refs.remove(&conn);
                    Ok(true) // fully removed
                } else {
                    *count -= 1;
                    Ok(false) // still has references
                }
            }
        }
    }

    fn force_remove(&mut self, conn: u64) -> Result<usize, DataPathError> {
        // Returns the count that was removed
        self.refs
            .remove(&conn)
            .ok_or(DataPathError::ConnectionIdNotFound(conn))
    }

    fn is_empty(&self) -> bool {
        self.refs.is_empty()
    }

    fn len(&self) -> usize {
        self.refs.len()
    }

    fn keys(&self) -> impl Iterator<Item = &u64> {
        self.refs.keys()
    }

    fn iter(&self) -> impl Iterator<Item = (&u64, &usize)> {
        self.refs.iter()
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
                    error!(index = %*pos, "error retrieving the connection from the pool");
                }
                Some(conn_id) => {
                    conn_id.counter += 1;
                }
            },
        }
    }

    fn remove(&mut self, conn: u64) -> Result<(), DataPathError> {
        let conn_index_opt = self.index.get(&conn);
        if conn_index_opt.is_none() {
            debug!(%conn, "cannot find the index for connection");
            return Err(DataPathError::ConnectionIdNotFound(conn));
        }
        let conn_index = conn_index_opt.unwrap();
        let conn_id_opt = self.pool.get_mut(*conn_index);
        if conn_id_opt.is_none() {
            debug!(%conn, "cannot find the connection in the pool");
            return Err(DataPathError::ConnectionIdNotFound(conn));
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
    // map name -> [local connection refs, remote connection refs]
    // the array contains the local connections at position 0 and the
    // remote ones at position 1
    // SubscriptionRefs tracks reference counts for each connection
    ids: HashMap<u64, [SubscriptionRefs; 2]>,
    // List of all the connections that are available for this name
    // as for the ids map position 0 stores local connections and position
    // 1 store remotes ones
    connections: [Connections; 2],
}

impl NameState {
    fn new(id: u64, conn: u64, is_local: bool) -> Self {
        let mut type_state = NameState::default();
        let refs = SubscriptionRefs::new(conn);
        if is_local {
            type_state.connections[0].insert(conn);
            type_state
                .ids
                .insert(id, [refs, SubscriptionRefs::default()]);
        } else {
            type_state.connections[1].insert(conn);
            type_state
                .ids
                .insert(id, [SubscriptionRefs::default(), refs]);
        }
        type_state
    }

    fn insert(&mut self, id: u64, conn: u64, is_local: bool) {
        let index = if is_local { 0 } else { 1 };
        self.connections[index].insert(conn);

        match self.ids.get_mut(&id) {
            None => {
                // the id does not exist
                let mut connections = [SubscriptionRefs::default(), SubscriptionRefs::default()];
                connections[index].insert(conn);
                self.ids.insert(id, connections);
            }
            Some(v) => {
                v[index].insert(conn);
            }
        }
    }

    fn remove(&mut self, id: &u64, conn: u64, is_local: bool) -> Result<bool, DataPathError> {
        // Returns true if the subscription was fully removed (ref count reached 0)
        match self.ids.get_mut(id) {
            None => {
                warn!(%id, "not found");
                Err(DataPathError::IdNotFound(*id))
            }
            Some(connection_refs) => {
                let index = if is_local { 0 } else { 1 };

                let fully_removed = connection_refs[index].remove(conn)?;
                self.connections[index].remove(conn)?;

                if fully_removed {
                    // if both refs are empty remove the id from the tables
                    if connection_refs[0].is_empty() && connection_refs[1].is_empty() {
                        self.ids.remove(id);
                    }
                }
                Ok(fully_removed)
            }
        }
    }

    fn force_remove(&mut self, id: &u64, conn: u64, is_local: bool) -> Result<(), DataPathError> {
        // Force remove regardless of counter - used when connection dies
        match self.ids.get_mut(id) {
            None => {
                warn!(%id, "not found");
                Err(DataPathError::IdNotFound(*id))
            }
            Some(connection_refs) => {
                let index = if is_local { 0 } else { 1 };

                let count = connection_refs[index].force_remove(conn)?;
                // Remove from connections pool, decrementing by the full count
                for _ in 0..count {
                    self.connections[index].remove(conn)?;
                }
                // if both refs are empty remove the id from the tables
                if connection_refs[0].is_empty() && connection_refs[1].is_empty() {
                    self.ids.remove(id);
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
                debug!(name = %id, "cannot find out connection, name does not exists");
                None
            }
            Some(refs) => {
                if refs[index].is_empty() {
                    // no connections available
                    return None;
                }

                if refs[index].len() == 1 {
                    // Get the single connection id
                    let conn_id = *refs[index].keys().next().unwrap();
                    if conn_id == incoming_conn {
                        // cannot return the incoming connection
                        debug!("the only available connection cannot be used");
                        return None;
                    } else {
                        return Some(conn_id);
                    }
                }

                // we need to iterate and find a value starting from a random point
                let conns: Vec<u64> = refs[index].keys().copied().collect();
                let mut rng = rand::rng();
                let pos = rng.random_range(0..conns.len());
                let mut stop = false;
                let mut i = pos;
                while !stop {
                    if conns[i] != incoming_conn {
                        return Some(conns[i]);
                    }
                    i = (i + 1) % conns.len();
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
                debug!(%id, "cannot find out connection, id does not exists");
                None
            }
            Some(refs) => {
                if refs[index].is_empty() {
                    // should never happen
                    return None;
                }

                if refs[index].len() == 1 {
                    // Get the single connection id
                    let conn_id = *refs[index].keys().next().unwrap();
                    if conn_id == incoming_conn {
                        // cannot return the incoming connection
                        debug!("the only available connection cannot be used");
                        return None;
                    } else {
                        return Some(vec![conn_id]);
                    }
                }

                // we need to iterate over the refs and exclude the incoming connection
                let mut out = Vec::new();
                for conn_id in refs[index].keys() {
                    if *conn_id != incoming_conn {
                        out.push(*conn_id);
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
            for (id, conn_refs) in v.ids.iter() {
                writeln!(f, "    Id: {}", id)?;
                if conn_refs[0].is_empty() {
                    writeln!(f, "       Local Connections:")?;
                    writeln!(f, "         None")?;
                } else {
                    writeln!(f, "       Local Connections:")?;
                    for (c, count) in conn_refs[0].iter() {
                        writeln!(f, "         Connection: {} (refs: {})", c, count)?;
                    }
                }
                if conn_refs[1].is_empty() {
                    writeln!(f, "       Remote Connections:")?;
                    writeln!(f, "         None")?;
                } else {
                    writeln!(f, "       Remote Connections:")?;
                    for (c, count) in conn_refs[1].iter() {
                        writeln!(f, "         Connection: {} (refs: {})", c, count)?;
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
                name = %internal_name.0, %conn,
                "subscription table: add first subscription",
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
) -> Result<(), DataPathError> {
    let name_str = name.to_string();

    let set = map.get_mut(&conn_index);
    match set {
        None => {
            debug!(
                name = %name_str, %conn_index,
                "add first subscription for name",
            );
            let mut set = HashSet::new();
            set.insert(name);
            map.insert(conn_index, set);
        }
        Some(s) => {
            debug!(
                name = %name_str, %conn_index, "add subscription",
            );

            if !s.insert(name) {
                // Subscription already exists in the set - this is expected with refcounting
                debug!(
                    name = %name_str,
                    %conn_index,
                    "subscription already tracked in connections set (refcount incremented)",
                );
                return Ok(());
            }
        }
    }
    debug!(
        name = %name_str, %conn_index,
        "subscription successfully added on connection",
    );
    Ok(())
}

fn remove_subscription_from_sub_table(
    name: &Name,
    conn_index: u64,
    is_local: bool,
    table: &mut RwLockWriteGuard<'_, RawRwLock, HashMap<InternalName, NameState>>,
) -> Result<bool, DataPathError> {
    // Returns true if the subscription was fully removed (ref count reached 0)
    // Convert &Name to &InternalName. This is unsafe, but we know the types are compatible.
    let query_name = unsafe { std::mem::transmute::<&Name, &InternalName>(name) };

    if let Some(state) = table.get_mut(query_name) {
        let fully_removed = state.remove(&name.id(), conn_index, is_local)?;

        if state.ids.is_empty() {
            table.remove(query_name);
        }
        Ok(fully_removed)
    } else {
        debug!("subscription not found {}", name);
        Err(DataPathError::SubscriptionNotFound(name.clone()))
    }
}

fn force_remove_subscription_from_sub_table(
    name: &Name,
    conn_index: u64,
    is_local: bool,
    table: &mut RwLockWriteGuard<'_, RawRwLock, HashMap<InternalName, NameState>>,
) -> Result<(), DataPathError> {
    // Convert &Name to &InternalName. This is unsafe, but we know the types are compatible.
    let query_name = unsafe { std::mem::transmute::<&Name, &InternalName>(name) };

    if let Some(state) = table.get_mut(query_name) {
        state.force_remove(&name.id(), conn_index, is_local)?;
        if state.ids.is_empty() {
            table.remove(query_name);
        }
        Ok(())
    } else {
        debug!("subscription not found {}", name);
        Err(DataPathError::SubscriptionNotFound(name.clone()))
    }
}

fn remove_subscription_from_connection(
    name: &Name,
    conn_index: u64,
    mut map: RwLockWriteGuard<'_, RawRwLock, HashMap<u64, HashSet<Name>>>,
) -> Result<(), DataPathError> {
    let set = map.get_mut(&conn_index);
    match set {
        None => {
            warn!(%conn_index, "connection not found");
            return Err(DataPathError::ConnectionIdNotFound(conn_index));
        }
        Some(s) => {
            if !s.remove(name) {
                warn!(
                    %name, %conn_index,
                    "subscription not found for connection",
                );
                return Err(DataPathError::SubscriptionNotFound(name.clone()));
            }
            if s.is_empty() {
                map.remove(&conn_index);
            }
        }
    }
    debug!(
        %name, %conn_index,
        "subscription successfully removed",
    );
    Ok(())
}

impl SubscriptionTable for SubscriptionTableImpl {
    type Error = DataPathError;

    fn for_each<F>(&self, mut f: F)
    where
        F: FnMut(&Name, u64, &[u64], &[u64]),
    {
        let table = self.table.read();

        for (k, v) in table.iter() {
            for (id, conn_refs) in v.ids.iter() {
                // Convert SubscriptionRefs keys to Vec for the callback
                let local: Vec<u64> = conn_refs[0].keys().copied().collect();
                let remote: Vec<u64> = conn_refs[1].keys().copied().collect();
                f(&k.0, *id, local.as_ref(), remote.as_ref());
            }
        }
    }

    fn add_subscription(&self, name: Name, conn: u64, is_local: bool) -> Result<(), Self::Error> {
        // Add to sub_table and increment counter if the subscription already exists
        {
            let table = self.table.write();
            add_subscription_to_sub_table(name.clone(), conn, is_local, table);
        }
        // Try to add to connections map (will fail silently if already exists)
        {
            let conn_table = self.connections.write();
            let _ = add_subscription_to_connection(name, conn, conn_table);
        }
        Ok(())
    }

    fn remove_subscription(
        &self,
        name: &Name,
        conn: u64,
        is_local: bool,
    ) -> Result<(), Self::Error> {
        let fully_removed = {
            let mut table = self.table.write();
            remove_subscription_from_sub_table(name, conn, is_local, &mut table)?
        };
        // Only remove from connections HashSet if subscription was fully removed (ref count = 0)
        if fully_removed {
            let conn_table = self.connections.write();
            remove_subscription_from_connection(name, conn, conn_table)?;
        }
        Ok(())
    }

    fn remove_connection(&self, conn: u64, is_local: bool) -> Result<HashSet<Name>, Self::Error> {
        let removed_subscriptions = self
            .connections
            .write()
            .remove(&conn)
            .ok_or(DataPathError::ConnectionIdNotFound(conn))?;
        let mut table = self.table.write();
        for name in &removed_subscriptions {
            debug!(%name, %conn, "remove subscription");
            // Use force_remove since the connection is dead
            force_remove_subscription_from_sub_table(name, conn, is_local, &mut table)?;
        }
        Ok(removed_subscriptions)
    }

    fn match_one(&self, name: &Name, incoming_conn: u64) -> Result<u64, Self::Error> {
        let table = self.table.read();

        let query_name = unsafe { std::mem::transmute::<&Name, &InternalName>(name) };

        match table.get(query_name) {
            None => {
                debug!(%name, "match not found for name");
                Err(DataPathError::NoMatch(name.clone()))
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
                debug!(%name, "no output connection available");
                Err(DataPathError::NoMatch(name.clone()))
            }
        }
    }

    fn match_all(&self, name: &Name, incoming_conn: u64) -> Result<Vec<u64>, Self::Error> {
        let table = self.table.read();

        let query_name = unsafe { std::mem::transmute::<&Name, &InternalName>(name) };

        match table.get(query_name) {
            None => {
                debug!(%name, "match not found for name");
                Err(DataPathError::NoMatch(name.clone()))
            }
            Some(state) => {
                let mut all_connections = Vec::new();

                // Collect local connections
                if let Some(local_out) = state.get_all_connections(name.id(), incoming_conn, true) {
                    debug!(?local_out, "found local connections");
                    all_connections.extend(local_out);
                }

                // Collect remote connections
                if let Some(remote_out) = state.get_all_connections(name.id(), incoming_conn, false)
                {
                    debug!(?remote_out, "found remote connections");
                    all_connections.extend(remote_out);
                }

                if all_connections.is_empty() {
                    debug!(%name, "no connection available (local/remote)");
                    Err(DataPathError::NoMatch(name.clone()))
                } else {
                    Ok(all_connections)
                }
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

        assert!(t.add_subscription(name1.clone(), 1, false).is_ok());
        assert!(t.add_subscription(name1.clone(), 2, false).is_ok());
        assert!(t.add_subscription(name1_1.clone(), 3, false).is_ok());
        assert!(t.add_subscription(name2_2.clone(), 3, false).is_ok());

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

        assert!(t.remove_subscription(&name1, 2, false).is_ok());

        // return two matches on connection 1,3
        let out = t.match_all(&name1, 100).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&1));
        assert!(out.contains(&3));

        assert!(t.remove_subscription(&name1_1, 3, false).is_ok());

        // return one matches on connection 1
        let out = t.match_all(&name1, 100).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        // return no match
        let err = t.match_all(&name1, 1);
        assert!(matches!(err, Err(DataPathError::NoMatch(_))));

        // add subscription again
        assert!(t.add_subscription(name1_1.clone(), 2, false).is_ok());

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

        assert!(t.add_subscription(name2_2.clone(), 4, false).is_ok());

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

        assert!(t.remove_subscription(&name2_2, 4, false).is_ok());

        // test local vs remote
        assert!(t.add_subscription(name1.clone(), 2, true).is_ok());

        // returns both local (2) and remote (1) connections
        let out = t.match_all(&name1, 100).unwrap();
        assert_eq!(out.len(), 2);
        assert!(out.contains(&2));
        assert!(out.contains(&1));

        // returns one match on connection 2
        let out = t.match_one(&name1, 100).unwrap();
        assert_eq!(out, 2);

        // returns both local (2) and remote (1) connections, excluding incoming connection (2)
        let out = t.match_all(&name1, 2).unwrap();
        assert_eq!(out.len(), 1);
        assert!(out.contains(&1));

        // same here
        let out = t.match_one(&name1, 2).unwrap();
        assert_eq!(out, 1);

        // test errors
        let err = t.remove_connection(4, false);
        assert!(matches!(err, Err(DataPathError::ConnectionIdNotFound(_))));

        assert_eq!(t.match_one(&name1_1, 100).unwrap(), 2);

        assert!(
            // this generates a warning
            t.add_subscription(name2_2.clone(), 3, false).is_ok()
        );

        let err = t.remove_subscription(&name3, 2, false);
        assert!(matches!(err, Err(DataPathError::SubscriptionNotFound(_))));

        let err = t.remove_subscription(&name2, 2, false);
        assert!(matches!(err, Err(DataPathError::IdNotFound(_))));
    }

    #[test]
    fn test_iter() {
        let name1 = Name::from_strings(["agntcy", "default", "one"]);
        let name2 = Name::from_strings(["agntcy", "default", "two"]);

        let t = SubscriptionTableImpl::default();

        assert!(t.add_subscription(name1.clone(), 1, false).is_ok());
        assert!(t.add_subscription(name1.clone(), 2, false).is_ok());
        assert!(t.add_subscription(name2.clone(), 3, true).is_ok());

        let mut h = HashMap::new();

        t.for_each(|k, id, local, remote| {
            println!(
                "key: {}, id: {}, local: {:?}, remote: {:?}",
                k, id, local, remote
            );

            let mut local_sorted = local.to_vec();
            local_sorted.sort();
            let mut remote_sorted = remote.to_vec();
            remote_sorted.sort();
            h.insert(k.clone(), (id, local_sorted, remote_sorted));
        });

        assert_eq!(h.len(), 2);
        assert_eq!(h[&name1].1, vec![] as Vec<u64>);
        assert_eq!(h[&name1].2, vec![1, 2]);

        assert_eq!(h[&name2].1, vec![3]);
        assert_eq!(h[&name2].2, vec![] as Vec<u64>);
    }

    #[test]
    fn test_match_all_with_mixed_local_and_remote_connections() {
        let name = Name::from_strings(["agntcy", "default", "service"]);
        let t = SubscriptionTableImpl::default();

        // Add local connections
        assert!(t.add_subscription(name.clone(), 1, true).is_ok());
        assert!(t.add_subscription(name.clone(), 2, true).is_ok());

        // Add remote connections
        assert!(t.add_subscription(name.clone(), 3, false).is_ok());
        assert!(t.add_subscription(name.clone(), 4, false).is_ok());

        // Test match_all returns both local and remote connections
        let result = t.match_all(&name, 100).unwrap();
        assert_eq!(
            result.len(),
            4,
            "Should return all 4 connections (2 local + 2 remote)"
        );
        assert!(result.contains(&1), "Should contain local connection 1");
        assert!(result.contains(&2), "Should contain local connection 2");
        assert!(result.contains(&3), "Should contain remote connection 3");
        assert!(result.contains(&4), "Should contain remote connection 4");

        // Test excluding incoming connection works for local
        let result = t.match_all(&name, 1).unwrap();
        assert_eq!(
            result.len(),
            3,
            "Should return 3 connections (excluding conn 1)"
        );
        assert!(
            !result.contains(&1),
            "Should not contain incoming connection 1"
        );
        assert!(result.contains(&2), "Should contain local connection 2");
        assert!(result.contains(&3), "Should contain remote connection 3");
        assert!(result.contains(&4), "Should contain remote connection 4");

        // Test excluding incoming connection works for remote
        let result = t.match_all(&name, 3).unwrap();
        assert_eq!(
            result.len(),
            3,
            "Should return 3 connections (excluding conn 3)"
        );
        assert!(result.contains(&1), "Should contain local connection 1");
        assert!(result.contains(&2), "Should contain local connection 2");
        assert!(
            !result.contains(&3),
            "Should not contain incoming connection 3"
        );
        assert!(result.contains(&4), "Should contain remote connection 4");

        // Test match_one prefers local over remote
        for _ in 0..20 {
            let result = t.match_one(&name, 100).unwrap();
            assert!(
                result == 1 || result == 2,
                "match_one should always prefer local connections"
            );
        }

        // Remove all local connections
        assert!(t.remove_subscription(&name, 1, true).is_ok());
        assert!(t.remove_subscription(&name, 2, true).is_ok());

        // Now match_all should only return remote connections
        let result = t.match_all(&name, 100).unwrap();
        assert_eq!(result.len(), 2, "Should return only 2 remote connections");
        assert!(result.contains(&3), "Should contain remote connection 3");
        assert!(result.contains(&4), "Should contain remote connection 4");

        // And match_one should fall back to remote
        for _ in 0..20 {
            let result = t.match_one(&name, 100).unwrap();
            assert!(
                result == 3 || result == 4,
                "Should return remote connections"
            );
        }
    }

    #[test]
    #[traced_test]
    fn test_subscription_refcounting() {
        let name1 = Name::from_strings(["agntcy", "default", "service"]);
        let t = SubscriptionTableImpl::default();

        // Add the same subscription multiple times
        assert!(t.add_subscription(name1.clone(), 1, false).is_ok());
        assert!(t.add_subscription(name1.clone(), 1, false).is_ok());
        assert!(t.add_subscription(name1.clone(), 1, false).is_ok());

        // Should still be able to match to connection 1
        let result = t.match_one(&name1, 100).unwrap();
        assert_eq!(result, 1, "Should match to connection 1");

        // Remove once - subscription should still exist (counter: 3 -> 2)
        assert!(t.remove_subscription(&name1, 1, false).is_ok());
        let result = t.match_one(&name1, 100).unwrap();
        assert_eq!(
            result, 1,
            "Should still match to connection 1 after first remove"
        );

        // Remove again - subscription should still exist (counter: 2 -> 1)
        assert!(t.remove_subscription(&name1, 1, false).is_ok());
        let result = t.match_one(&name1, 100).unwrap();
        assert_eq!(
            result, 1,
            "Should still match to connection 1 after second remove"
        );

        // Remove final time - subscription should now be fully removed (counter: 1 -> 0)
        assert!(t.remove_subscription(&name1, 1, false).is_ok());
        let err = t.match_one(&name1, 100);
        assert!(
            matches!(err, Err(DataPathError::NoMatch(_))),
            "Should have no match after final remove"
        );

        // Test with multiple connections having different ref counts
        let name2 = Name::from_strings(["agntcy", "default", "multi"]);

        // Connection 1: ref count 3
        assert!(t.add_subscription(name2.clone(), 1, false).is_ok());
        assert!(t.add_subscription(name2.clone(), 1, false).is_ok());
        assert!(t.add_subscription(name2.clone(), 1, false).is_ok());

        // Connection 2: ref count 1
        assert!(t.add_subscription(name2.clone(), 2, false).is_ok());

        // Connection 3: ref count 2
        assert!(t.add_subscription(name2.clone(), 3, false).is_ok());
        assert!(t.add_subscription(name2.clone(), 3, false).is_ok());

        // All three connections should be available
        let result = t.match_all(&name2, 100).unwrap();
        assert_eq!(result.len(), 3, "Should have 3 connections");
        assert!(result.contains(&1));
        assert!(result.contains(&2));
        assert!(result.contains(&3));

        // Remove connection 2 once - should be gone (ref count 1 -> 0)
        assert!(t.remove_subscription(&name2, 2, false).is_ok());
        let result = t.match_all(&name2, 100).unwrap();
        assert_eq!(
            result.len(),
            2,
            "Should have 2 connections after removing conn 2"
        );
        assert!(!result.contains(&2), "Connection 2 should be removed");

        // Remove connection 1 once - should still exist (ref count 3 -> 2)
        assert!(t.remove_subscription(&name2, 1, false).is_ok());
        let result = t.match_all(&name2, 100).unwrap();
        assert_eq!(result.len(), 2, "Should still have 2 connections");
        assert!(result.contains(&1), "Connection 1 should still exist");

        // Remove connection 3 twice - should be gone (ref count 2 -> 0)
        assert!(t.remove_subscription(&name2, 3, false).is_ok());
        assert!(t.remove_subscription(&name2, 3, false).is_ok());
        let result = t.match_all(&name2, 100).unwrap();
        assert_eq!(result.len(), 1, "Should have only 1 connection");
        assert!(result.contains(&1), "Only connection 1 should remain");
    }

    #[test]
    #[traced_test]
    fn test_connection_death_with_refcounting() {
        let name1 = Name::from_strings(["agntcy", "default", "cleanup"]);
        let t = SubscriptionTableImpl::default();

        // Add subscription multiple times
        assert!(t.add_subscription(name1.clone(), 1, false).is_ok());
        assert!(t.add_subscription(name1.clone(), 1, false).is_ok());
        assert!(t.add_subscription(name1.clone(), 1, false).is_ok());

        // Add another connection with single ref
        assert!(t.add_subscription(name1.clone(), 2, false).is_ok());

        // Both connections should be available
        let result = t.match_all(&name1, 100).unwrap();
        assert_eq!(result.len(), 2, "Should have 2 connections");

        // Connection 1 dies - should be force-removed regardless of ref count
        let removed = t.remove_connection(1, false).unwrap();
        assert_eq!(removed.len(), 1, "Should have removed 1 subscription");
        assert!(removed.contains(&name1));

        // Now only connection 2 should be available
        let result = t.match_one(&name1, 100).unwrap();
        assert_eq!(
            result, 2,
            "Should only match to connection 2 after conn 1 dies"
        );

        let result = t.match_all(&name1, 100).unwrap();
        assert_eq!(result.len(), 1, "Should only have 1 connection remaining");
        assert!(result.contains(&2));
    }

    #[test]
    #[traced_test]
    fn test_mixed_local_remote_refcounting() {
        let name1 = Name::from_strings(["agntcy", "default", "mixed"]);
        let t = SubscriptionTableImpl::default();

        // Add local connection multiple times
        assert!(t.add_subscription(name1.clone(), 1, true).is_ok());
        assert!(t.add_subscription(name1.clone(), 1, true).is_ok());

        // Add remote connection multiple times
        assert!(t.add_subscription(name1.clone(), 2, false).is_ok());
        assert!(t.add_subscription(name1.clone(), 2, false).is_ok());
        assert!(t.add_subscription(name1.clone(), 2, false).is_ok());

        // Should prefer local connection
        for _ in 0..10 {
            let result = t.match_one(&name1, 100).unwrap();
            assert_eq!(result, 1, "Should prefer local connection");
        }

        // Remove local once - should still exist
        assert!(t.remove_subscription(&name1, 1, true).is_ok());
        let result = t.match_one(&name1, 100).unwrap();
        assert_eq!(result, 1, "Local connection should still exist");

        // Remove local again - should be gone, fall back to remote
        assert!(t.remove_subscription(&name1, 1, true).is_ok());
        for _ in 0..10 {
            let result = t.match_one(&name1, 100).unwrap();
            assert_eq!(result, 2, "Should fall back to remote connection");
        }

        // Remove remote twice - should still exist (ref count 3 -> 1)
        assert!(t.remove_subscription(&name1, 2, false).is_ok());
        assert!(t.remove_subscription(&name1, 2, false).is_ok());
        let result = t.match_one(&name1, 100).unwrap();
        assert_eq!(result, 2, "Remote should still exist with ref count 1");

        // Remove remote final time - should be gone
        assert!(t.remove_subscription(&name1, 2, false).is_ok());
        let err = t.match_one(&name1, 100);
        assert!(
            matches!(err, Err(DataPathError::NoMatch(_))),
            "No connections should remain"
        );
    }
}
