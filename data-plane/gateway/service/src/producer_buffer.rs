// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;

use agp_datapath::pubsub::proto::pubsub::v1::Message;
use parking_lot::RwLock;

struct ProducerBufferImpl {
    capacity: usize,
    next: usize,
    buffer: Vec<Option<Message>>,
    map: HashMap<usize, usize>,
}

pub struct ProducerBuffer {
    buffer: RwLock<ProducerBufferImpl>,
}

impl ProducerBufferImpl {
    /// Create a buffer with a given capacity
    fn with_capacity(capacity: usize) -> Self {
        ProducerBufferImpl {
            capacity,
            next: 0,
            buffer: vec![None; capacity],
            map: HashMap::new(),
        }
    }

    fn get_capacity(&self) -> usize {
        self.capacity
    }

    /// Add message to the buffer.
    /// return true if the insertion completes
    fn push(&mut self, msg: Message) -> bool {
        // get message id
        let id = msg.get_id() as usize;

        // check if the message is already there
        // if yes return
        if self.map.contains_key(&id) {
            return true;
        }

        // remove the message at position next from the map
        // the same message will be overwritten in the buffer
        if let Some(message) = &self.buffer[self.next] {
            let to_remove = message.get_id() as usize;
            self.map.remove(&to_remove);
        }

        // store the new message
        self.buffer[self.next] = Some(msg);
        // store the position of the message in the buffer
        self.map.insert(id, self.next);
        // increase the index to the next element in the buffer
        self.next = (self.next + 1) % self.capacity;
        true
    }

    /// Remove all the elements in the buffer
    fn clear(&mut self) {
        self.buffer = vec![None; self.capacity];
        self.next = 0;
        self.map.clear();
    }

    fn get(&self, id: usize) -> Option<Message> {
        match self.map.get(&id) {
            None => None,
            Some(index) => self.buffer[*index].clone(),
        }
    }
}

impl ProducerBuffer {
    pub fn with_capacity(capacity: usize) -> Self {
        ProducerBuffer {
            buffer: ProducerBufferImpl::with_capacity(capacity).into(),
        }
    }

    pub fn get_capacity(&self) -> usize {
        let lock = self.buffer.read();
        lock.get_capacity()
    }

    pub fn push(&self, msg: Message) -> bool {
        let mut lock = self.buffer.write();
        lock.push(msg)
    }

    pub fn clear(&self) {
        let mut lock = self.buffer.write();
        lock.clear()
    }

    pub fn get(&self, id: usize) -> Option<Message> {
        let lock = self.buffer.read();
        lock.get(id)
    }
}

// tests
#[cfg(test)]
mod tests {
    use super::*;
    use agp_datapath::messages::encoder::{Agent, AgentType};
    use agp_datapath::pubsub::proto::pubsub::v1::SessionHeaderType;
    use agp_datapath::pubsub::{AgpHeader, SessionHeader};

    #[test]
    fn test_producer_buffer() {
        let buffer = ProducerBuffer::with_capacity(3);

        assert_eq!(buffer.get_capacity(), 3);

        let src = Agent::from_strings("org", "ns", "type", 0);
        let name_type = AgentType::from_strings("org", "ns", "type");

        let agp_header = AgpHeader::new(&src, &name_type, Some(1), None);

        let h0 = SessionHeader::new(SessionHeaderType::Fnf.into(), 0, 0);
        let h1 = SessionHeader::new(SessionHeaderType::Fnf.into(), 0, 1);
        let h2 = SessionHeader::new(SessionHeaderType::Fnf.into(), 0, 2);
        let h3 = SessionHeader::new(SessionHeaderType::Fnf.into(), 0, 3);
        let h4 = SessionHeader::new(SessionHeaderType::Fnf.into(), 0, 4);

        let p0 = Message::new_publish_with_headers(Some(agp_header), Some(h0), "", vec![]);
        let p1 = Message::new_publish_with_headers(Some(agp_header), Some(h1), "", vec![]);
        let p2 = Message::new_publish_with_headers(Some(agp_header), Some(h2), "", vec![]);
        let p3 = Message::new_publish_with_headers(Some(agp_header), Some(h3), "", vec![]);
        let p4 = Message::new_publish_with_headers(Some(agp_header), Some(h4), "", vec![]);

        assert_eq!(buffer.push(p0.clone()), true);

        assert_eq!(buffer.get(0).unwrap(), p0);
        assert_eq!(buffer.get(0).unwrap(), p0);
        assert_eq!(buffer.get(0).unwrap(), p0);
        assert_eq!(buffer.get(1), None);

        assert_eq!(buffer.push(p0.clone()), true);
        assert_eq!(buffer.push(p1.clone()), true);
        assert_eq!(buffer.push(p2.clone()), true);

        assert_eq!(buffer.get(0).unwrap(), p0);
        assert_eq!(buffer.get(1).unwrap(), p1);
        assert_eq!(buffer.get(2).unwrap(), p2);
        assert_eq!(buffer.get(3), None);

        // now the buffer is full, add a new element will remote the elem 0
        assert_eq!(buffer.push(p3.clone()), true);
        assert_eq!(buffer.get(0), None);
        assert_eq!(buffer.get(1).unwrap(), p1);
        assert_eq!(buffer.get(2).unwrap(), p2);
        assert_eq!(buffer.get(3).unwrap(), p3);
        assert_eq!(buffer.get(4), None);

        // now the buffer is full, add a new element will remote the elem 1
        assert_eq!(buffer.push(p4.clone()), true);
        assert_eq!(buffer.get(0), None);
        assert_eq!(buffer.get(1), None);
        assert_eq!(buffer.get(2).unwrap(), p2);
        assert_eq!(buffer.get(3).unwrap(), p3);
        assert_eq!(buffer.get(4).unwrap(), p4);

        // remove all elements
        buffer.clear();
        assert_eq!(buffer.get(0), None);
        assert_eq!(buffer.get(1), None);
        assert_eq!(buffer.get(2), None);
        assert_eq!(buffer.get(3), None);
        assert_eq!(buffer.get(4), None);

        // add all msgs and check again
        assert_eq!(buffer.push(p0.clone()), true);
        assert_eq!(buffer.push(p1.clone()), true);
        assert_eq!(buffer.push(p2.clone()), true);
        assert_eq!(buffer.push(p3.clone()), true);
        assert_eq!(buffer.push(p4.clone()), true);
        assert_eq!(buffer.get(0), None);
        assert_eq!(buffer.get(1), None);
        assert_eq!(buffer.get(2).unwrap(), p2);
        assert_eq!(buffer.get(3).unwrap(), p3);
        assert_eq!(buffer.get(4).unwrap(), p4);
    }
}
