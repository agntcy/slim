// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::HashMap;

// Third-party crates
use slim_datapath::{api::ProtoMessage as Message, api::ProtoName};

pub struct ProducerBuffer {
    capacity: usize,
    next: usize,
    messages: HashMap<usize, Message>, // message_id → Message
    ring: Vec<usize>,                  // ring of IDs in insertion order; usize::MAX = empty slot
    destination_name: ProtoName,
    destination_id: Option<u64>,
}

impl ProducerBuffer {
    /// Create a buffer with a given capacity
    pub fn with_capacity(capacity: usize) -> Self {
        ProducerBuffer {
            capacity,
            next: 0,
            messages: HashMap::with_capacity(capacity),
            ring: vec![usize::MAX; capacity],
            destination_name: ProtoName::from_strings(["unknown", "unknown", "unknown"]),
            destination_id: None,
        }
    }

    pub fn get_capacity(&self) -> usize {
        self.capacity
    }

    pub fn get_destination_name(&self) -> &ProtoName {
        &self.destination_name
    }

    pub fn get_destination_id(&self) -> Option<u64> {
        self.destination_id
    }

    /// Add message to the buffer.
    pub fn push(&mut self, msg: Message) {
        // if messages is empty init the destination name
        if self.messages.is_empty() {
            self.destination_name = msg.get_dst();
        }

        let id = msg.get_id() as usize;

        // check if the message is already there; if yes return
        if self.messages.contains_key(&id) {
            return;
        }

        // Evict the oldest entry: read its ID from the compact ring Vec (no Message access).
        // messages.remove is a no-op when the slot holds the sentinel or an already-evicted ID.
        let evicted_id = self.ring[self.next];
        self.messages.remove(&evicted_id);

        // Store new message and record its ID in the ring.
        self.messages.insert(id, msg);
        self.ring[self.next] = id;
        self.next = (self.next + 1) % self.capacity;
    }

    /// Remove all the elements in the buffer
    pub fn clear(&mut self) {
        self.messages.clear();
        self.next = 0;
        // Stale IDs in `ring` are harmless: they are no longer in `messages` after clear(),
        // so messages.remove is a no-op and messages.get returns None until overwritten.
    }

    pub fn get(&self, id: usize) -> Option<Message> {
        self.messages.get(&id).cloned()
    }

    /// Iterate messages in insertion order (oldest → newest).
    pub fn iter(&self) -> impl Iterator<Item = &Message> + '_ {
        let start = self.next;
        (0..self.capacity)
            .map(move |i| (start + i) % self.capacity)
            .filter_map(move |pos| self.messages.get(&self.ring[pos]))
    }
}

// tests
#[cfg(test)]
mod tests {
    use super::*;
    use slim_datapath::api::ProtoName as Name;
    use slim_datapath::api::{
        ProtoSessionMessageType, ProtoSessionType, SessionHeader, SlimHeader,
    };

    #[test]
    fn test_producer_messages() {
        let mut messages = ProducerBuffer::with_capacity(3);

        assert_eq!(messages.get_capacity(), 3);

        let src = Name::from_strings(["org", "ns", "type"]).with_id(0);
        let src_id = src.to_string();
        let name_type = Name::from_strings(["org", "ns", "type"]).with_id(1);

        let slim_header = SlimHeader::new(src, name_type, &src_id, None);

        let h0 = SessionHeader::new(
            ProtoSessionType::PointToPoint.into(),
            ProtoSessionMessageType::Msg.into(),
            0,
            0,
        );
        let h1 = SessionHeader::new(
            ProtoSessionType::PointToPoint.into(),
            ProtoSessionMessageType::Msg.into(),
            0,
            1,
        );
        let h2 = SessionHeader::new(
            ProtoSessionType::PointToPoint.into(),
            ProtoSessionMessageType::Msg.into(),
            0,
            2,
        );
        let h3 = SessionHeader::new(
            ProtoSessionType::PointToPoint.into(),
            ProtoSessionMessageType::Msg.into(),
            0,
            3,
        );
        let h4 = SessionHeader::new(
            ProtoSessionType::PointToPoint.into(),
            ProtoSessionMessageType::Msg.into(),
            0,
            4,
        );

        let p0 = Message::builder()
            .with_slim_header(slim_header.clone())
            .with_session_header(h0)
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        let p1 = Message::builder()
            .with_slim_header(slim_header.clone())
            .with_session_header(h1)
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        let p2 = Message::builder()
            .with_slim_header(slim_header.clone())
            .with_session_header(h2)
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        let p3 = Message::builder()
            .with_slim_header(slim_header.clone())
            .with_session_header(h3)
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        let p4 = Message::builder()
            .with_slim_header(slim_header.clone())
            .with_session_header(h4)
            .application_payload("", vec![])
            .build_publish()
            .unwrap();

        assert!(messages.push(p0.clone()));

        assert_eq!(messages.get(0).unwrap(), p0);
        assert_eq!(messages.get(0).unwrap(), p0);
        assert_eq!(messages.get(0).unwrap(), p0);
        assert_eq!(messages.get(1), None);

        assert!(messages.push(p0.clone()));
        assert!(messages.push(p1.clone()));
        assert!(messages.push(p2.clone()));

        assert_eq!(messages.get(0).unwrap(), p0);
        assert_eq!(messages.get(1).unwrap(), p1);
        assert_eq!(messages.get(2).unwrap(), p2);
        assert_eq!(messages.get(3), None);

        // now the messages is full, add a new element will remote the elem 0
        assert!(messages.push(p3.clone()));
        assert_eq!(messages.get(0), None);
        assert_eq!(messages.get(1).unwrap(), p1);
        assert_eq!(messages.get(2).unwrap(), p2);
        assert_eq!(messages.get(3).unwrap(), p3);
        assert_eq!(messages.get(4), None);

        // now the messages is full, add a new element will remote the elem 1
        assert!(messages.push(p4.clone()));
        assert_eq!(messages.get(0), None);
        assert_eq!(messages.get(1), None);
        assert_eq!(messages.get(2).unwrap(), p2);
        assert_eq!(messages.get(3).unwrap(), p3);
        assert_eq!(messages.get(4).unwrap(), p4);

        // remove all elements
        messages.clear();
        assert_eq!(messages.get(0), None);
        assert_eq!(messages.get(1), None);
        assert_eq!(messages.get(2), None);
        assert_eq!(messages.get(3), None);
        assert_eq!(messages.get(4), None);

        // add all msgs and check again
        assert!(messages.push(p0.clone()));
        assert!(messages.push(p1.clone()));
        assert!(messages.push(p2.clone()));
        assert!(messages.push(p3.clone()));
        assert!(messages.push(p4.clone()));
        assert_eq!(messages.get(0), None);
        assert_eq!(messages.get(1), None);
        assert_eq!(messages.get(2).unwrap(), p2);
        assert_eq!(messages.get(3).unwrap(), p3);
        assert_eq!(messages.get(4).unwrap(), p4);
    }

    #[test]
    fn test_iter_producer_messages() {
        let src = Name::from_strings(["org", "ns", "type"]).with_id(0);
        let src_id = src.to_string();
        let name_type = Name::from_strings(["org", "ns", "type"]).with_id(1);

        let slim_header = SlimHeader::new(src, name_type, &src_id, None);
        let h = SessionHeader::new(
            ProtoSessionType::PointToPoint.into(),
            ProtoSessionMessageType::Msg.into(),
            0,
            0,
        );
        let mut p = Message::builder()
            .with_slim_header(slim_header)
            .with_session_header(h)
            .application_payload("", vec![])
            .build_publish()
            .unwrap();

        let mut b = ProducerBuffer::with_capacity(30);
        b.push(p.clone()); // add 0
        p.set_message_id(1);
        b.push(p.clone()); // add 1
        p.set_message_id(2);
        b.push(p.clone()); // add 2
        p.set_message_id(5);
        b.push(p.clone()); // add 5
        p.set_message_id(6);
        b.push(p.clone()); // add 6
        p.set_message_id(10);
        b.push(p.clone()); // add 10

        let expected = [0, 1, 2, 5, 6, 10];

        for (i, m) in b.iter().enumerate() {
            assert_eq!(m.get_id(), expected[i]);
        }
    }
}
