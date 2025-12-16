// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::collections::HashSet;

// Third-party crates
use tracing::{debug, info, trace};

use slim_datapath::{api::ProtoMessage as Message, messages::utils::MAX_PUBLISH_ID};

pub(crate) struct ReceiverBuffer {
    // ID of the last packet sent to the application
    // Init to usize max and it takes the values of the first
    // packet received in the buffer
    last_sent: usize,
    // First valid entry in the buffer. Packets may be
    // removed from the front of the buffer and we want to
    // avoid copies. This pointer keeps track of the valid entries
    first_entry: usize,
    // set of messages definitely lost that cannot be recoverd
    // anymore using RTX messages
    lost_msgs: HashSet<usize>,
    // Buffer of valid messages received Out-of-Order (OOO)
    // waiting to be delivered to the application
    // The first valid entry of the buffer always corresponds to
    // last_sent + 1
    buffer: Vec<Option<Message>>,
}

impl Default for ReceiverBuffer {
    fn default() -> Self {
        ReceiverBuffer {
            last_sent: usize::MAX,
            first_entry: 0,
            lost_msgs: HashSet::new(),
            buffer: vec![],
        }
    }
}

impl ReceiverBuffer {
    const MAX_ID: usize = MAX_PUBLISH_ID as usize;

    // Helper function to compute distance between two IDs with wraparound
    // Returns the forward distance from 'from' to 'to'
    fn id_distance(from: usize, to: usize) -> usize {
        if to >= from {
            to - from
        } else {
            // Wraparound case: from is near MAX_ID, to wrapped around
            (Self::MAX_ID + 1 - from) + to
        }
    }

    // Helper function to check if 'a' comes after 'b' in sequence (accounting for wraparound)
    // Returns true if 'a' is sequentially after 'b' within a reasonable window
    fn is_after(a: usize, b: usize) -> bool {
        let distance = Self::id_distance(b, a);
        // If distance is more than half the sequence space, assume it's actually before
        distance > 0 && distance <= Self::MAX_ID.div_ceil(2)
    }

    // Helper function to add offset to ID with wraparound
    fn add_with_wraparound(base: usize, offset: usize) -> usize {
        (base + offset) % (Self::MAX_ID + 1)
    }

    // returns a vec of messages to send to the application
    // in case the vector contains a None it means that the packet is lost
    // and cannot be recovered. the second vector contains the ids of the
    // packets lost that requires an RTX. If both vectors are empty the
    // caller has nothing to do
    pub fn on_received_message(&mut self, msg: Message) -> (Vec<Option<Message>>, Vec<u32>) {
        self.internal_on_received_message(msg.get_id() as usize, Some(msg))
    }

    // returns a list of messages that we can return to the application
    pub fn on_lost_message(&mut self, msg_id: u32) -> Vec<Option<Message>> {
        debug!(id = %msg_id, "message definitely lost");
        self.lost_msgs.insert(msg_id as usize);
        self.release_msgs()
    }

    // returns a list of lost messages for which RTX needs to be sent
    #[allow(dead_code)]
    pub fn on_beacon_message(&mut self, msg_id: u32) -> Vec<u32> {
        debug!(id = %msg_id,"received beacon");
        let (_recv, rtx) = self.internal_on_received_message(msg_id as usize, None);
        rtx
    }

    #[allow(dead_code)]
    pub fn message_already_received(&self, msg_id: usize) -> bool {
        if self.last_sent == usize::MAX {
            // no message received yet
            return false;
        }

        if !Self::is_after(msg_id, self.last_sent) {
            // this message was already delivered to the app
            // or it is impossible to recover it, no need
            // for RTX messages
            return true;
        }

        let next_expected = Self::add_with_wraparound(self.last_sent, 1);
        let buffer_end =
            Self::add_with_wraparound(self.last_sent, self.buffer.len() - self.first_entry);

        if self.buffer.is_empty()
            || !Self::is_after(msg_id, self.last_sent)
            || Self::is_after(msg_id, buffer_end)
        {
            // in this case the buffer is empty or the message id is outside the buffer range
            // this packet is still missing
            return false;
        }

        // the message is in the buffer range, check if it was received or not
        let pos = Self::id_distance(next_expected, msg_id) + self.first_entry;
        self.buffer[pos].is_some()
    }

    fn internal_on_received_message(
        &mut self,
        msg_id: usize,
        msg: Option<Message>,
    ) -> (Vec<Option<Message>>, Vec<u32>) {
        debug!(id = %msg_id, "Received message");

        let next_expected = if self.last_sent == usize::MAX {
            usize::MAX // Special value indicating no message received yet
        } else {
            Self::add_with_wraparound(self.last_sent, 1)
        };

        if self.last_sent == usize::MAX || (msg_id == next_expected) && (self.buffer.is_empty()) {
            match msg {
                Some(m) => {
                    // no loss detected, return message
                    // if this is the first packet received (case last_sent == usize::MAX) we consider it a
                    // valid one and the buffer is initialized accordingly. in this way a stream can start from
                    // a random number or it can be joined at any time
                    debug!(id = %msg_id, "No loss detected, return message");
                    self.last_sent = msg_id;
                    return (vec![Some(m)], vec![]);
                }
                None => {
                    // msg_id was lost
                    return (vec![], vec![msg_id as u32]);
                }
            }
        }

        // the message is an OOO check what to do with the message
        if !Self::is_after(msg_id, self.last_sent) {
            // this message is not useful anymore because we have already sent
            // content for this ID to the application. It can be a duplicated
            // msg or a message that arrived too late. Log and drop
            debug!("Received possibly DUP message or beacon for a received message, drop it");
            return (vec![], vec![]);
        }

        if self.buffer.is_empty() {
            // init the buffer and send required rtx
            self.first_entry = 0;
            // fill the buffer with an empty entry for each hole
            // detected in the message stream
            let mut rtx: Vec<u32> = Vec::new();
            let next_expected = Self::add_with_wraparound(self.last_sent, 1);
            let num_missing = Self::id_distance(next_expected, msg_id);

            match msg {
                Some(m) => {
                    self.buffer = vec![None; num_missing];
                    debug!(losses = %self.buffer.len(), "Losses found");
                    self.buffer.push(Some(m));
                    // Add RTX requests for all missing packets
                    let mut current = next_expected;
                    for _ in 0..num_missing {
                        trace!(%current, "add to rtx vector");
                        rtx.push(current as u32);
                        current = Self::add_with_wraparound(current, 1);
                    }
                }
                None => {
                    // we got a beacon message so we miss also msg_id
                    self.buffer = vec![None; num_missing + 1];
                    debug!(losses = %self.buffer.len(), "Losses found");
                    // Add RTX requests for all missing packets including msg_id
                    let mut current = next_expected;
                    for _ in 0..=num_missing {
                        trace!(%current, "add to rtx vector");
                        rtx.push(current as u32);
                        current = Self::add_with_wraparound(current, 1);
                    }
                }
            }
            (vec![], rtx)
        } else {
            debug!(
                id = %msg_id,
                "buffer is not empty and received OOO packet, process it",
            );
            trace!(
                last_sent = %self.last_sent,
                first_entry = %self.first_entry,
                len = %self.buffer.len(),
                "buffer status",
            );

            let next_expected = Self::add_with_wraparound(self.last_sent, 1);
            let buffer_end =
                Self::add_with_wraparound(self.last_sent, self.buffer.len() - self.first_entry);

            // check if the msg_id fits inside the buffer range
            let msg_in_range =
                Self::is_after(msg_id, self.last_sent) && !Self::is_after(msg_id, buffer_end);

            if msg_in_range {
                debug!(
                    id = %msg_id,
                    %next_expected,
                    %buffer_end,
                    "message is inside the buffer range",
                );
                // if mgs is None there is nothing to do here
                if msg.is_none() {
                    return (vec![], vec![]);
                }

                // find the position of the message in the buffer
                let pos = Self::id_distance(next_expected, msg_id) + self.first_entry;
                debug!(%msg_id, %pos, "try to insert message");
                if self.buffer[pos].is_some() {
                    // this is a duplicate message, drop it and do nothing
                    info!("Received DUP message, drop it");
                    return (vec![], vec![]);
                }
                debug!(
                    %msg_id,
                    %pos,
                    "add message and try to release msgs",
                );
                // add the message to the buffer and check if it is possible
                // to send some message to the application
                self.buffer[pos] = msg;

                // return the messages if possible
                (self.release_msgs(), vec![])
            } else {
                // the message is out of the current buffer
                // add more entries to it and return an empty vec
                let mut rtx = Vec::new();
                let buffer_next = Self::add_with_wraparound(buffer_end, 1);
                let num_new_entries = Self::id_distance(buffer_next, msg_id);

                let mut current = buffer_next;
                for _ in 0..num_new_entries {
                    self.buffer.push(None);
                    rtx.push(current as u32);
                    debug!(
                        %current,
                        "detect packet loss to add at the end of the buffer",
                    );
                    current = Self::add_with_wraparound(current, 1);
                }

                match msg {
                    Some(m) => {
                        debug!(id = %msg_id,"add packet at the end of the buffer");
                        self.buffer.push(Some(m));
                    }
                    None => {
                        // msg_id itself is lost, add it to the rtx list
                        rtx.push(msg_id as u32)
                    }
                }
                (vec![], rtx)
            }
        }
    }

    fn release_msgs(&mut self) -> Vec<Option<Message>> {
        let mut i = self.first_entry;
        let mut ret = vec![];
        while i < self.buffer.len() {
            if self.buffer[i].is_some() {
                // this message can be sent to the app
                ret.push(self.buffer[i].take());
                // increase last_sent with wraparound
                self.last_sent = Self::add_with_wraparound(self.last_sent, 1);
                self.first_entry += 1;
                debug!(
                    pos = %i,
                    last_sent = %self.last_sent,
                    first_index = %self.first_entry,
                    "return message at pos, new buffer state",
                );
            } else {
                // check if the msg id is in the set of lost messages
                let next_id = Self::add_with_wraparound(self.last_sent, 1);
                if self.lost_msgs.contains(&next_id) {
                    // this message cannot be recovered anymore
                    // add a None in the ret vec and release it
                    ret.push(None);
                    self.lost_msgs.remove(&next_id);
                    // increase all counters anyway because this
                    // position of the buffer will not be used anymore
                    self.last_sent = next_id;
                    self.first_entry += 1;
                    debug!(
                        lost_message = self.last_sent,
                        last_sent = %self.last_sent,
                        first_index = %self.first_entry,
                        "message lost, return none, new buffer state",
                    );
                } else {
                    // we need to wait a bit more
                    break;
                }
            }
            i += 1;
        }
        // check if the buffer is now empty
        if self.first_entry == self.buffer.len() {
            debug!("clean reception buffer which is empty now");
            // reset the buffer
            self.first_entry = 0;
            self.buffer = vec![];
        }
        // check if the next message in line is in the lost set
        // this should never happen in reality
        let mut stop = false;
        while !stop {
            let next_id = Self::add_with_wraparound(self.last_sent, 1);
            if self.lost_msgs.contains(&next_id) {
                self.last_sent = next_id;
                ret.push(None);
                self.lost_msgs.remove(&self.last_sent);
                debug!(
                    last_sent = %self.last_sent,
                    "found another lost message to release",
                );
            } else {
                stop = true;
            }
        }
        ret
    }
}

// tests
#[cfg(test)]
mod tests {
    use slim_datapath::api::{
        ProtoSessionMessageType, ProtoSessionType, SessionHeader, SlimHeader,
    };
    use slim_datapath::messages::encoder::Name;
    use tracing_test::traced_test;

    use super::*;

    #[test]
    #[traced_test]
    fn test_receiver_buffer() {
        let src = Name::from_strings(["org", "ns", "type"]).with_id(0);
        let src_id = src.to_string();
        let name_type = Name::from_strings(["org", "ns", "type"]).with_id(1);

        let slim_header = SlimHeader::new(&src, &name_type, &src_id, None);

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
        let h5 = SessionHeader::new(
            ProtoSessionType::PointToPoint.into(),
            ProtoSessionMessageType::Msg.into(),
            0,
            5,
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
        let p5 = Message::builder()
            .with_slim_header(slim_header.clone())
            .with_session_header(h5)
            .application_payload("", vec![])
            .build_publish()
            .unwrap();

        // insert in order
        let mut buffer = ReceiverBuffer::default();

        let (recv, rtx) = buffer.on_received_message(p0.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p0.clone()));

        let (recv, rtx) = buffer.on_received_message(p1.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p1.clone()));

        let (recv, rtx) = buffer.on_received_message(p2.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p2.clone()));

        let (recv, rtx) = buffer.on_received_message(p3.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p3.clone()));

        let (recv, rtx) = buffer.on_received_message(p4.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p4.clone()));

        // insert in order but skip first packets
        let mut buffer = ReceiverBuffer::default();

        let (recv, rtx) = buffer.on_received_message(p2.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p2.clone()));

        let (recv, rtx) = buffer.on_received_message(p3.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p3.clone()));

        let (recv, rtx) = buffer.on_received_message(p4.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p4.clone()));

        // receive DUP packets and old packets
        let mut buffer = ReceiverBuffer::default();

        let (recv, rtx) = buffer.on_received_message(p4.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p4.clone()));

        let (recv, rtx) = buffer.on_received_message(p4.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 0);

        let (recv, rtx) = buffer.on_received_message(p0.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 0);

        // insertion order 1, 4, 4, 2, 2, 3
        let mut buffer = ReceiverBuffer::default();

        // release 1
        let (recv, rtx) = buffer.on_received_message(p1.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p1.clone()));

        // detect loss for 2 and 3
        let (recv, rtx) = buffer.on_received_message(p4.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 2);
        assert_eq!(rtx[0], 2);
        assert_eq!(rtx[1], 3);

        // DUP packet, return nothing
        let (recv, rtx) = buffer.on_received_message(p4.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 0);

        // release packet 2
        let (recv, rtx) = buffer.on_received_message(p2.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p2.clone()));

        // Old packet, return nothing
        let (recv, rtx) = buffer.on_received_message(p2.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 0);

        // release packet 3 and 4
        let (recv, rtx) = buffer.on_received_message(p3.clone());
        assert_eq!(recv.len(), 2);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p3.clone()));
        assert_eq!(recv[1], Some(p4.clone()));

        // insertion order 0, 2, 5, 2, 3, 4, 1
        let mut buffer = ReceiverBuffer::default();

        // release 0
        let (recv, rtx) = buffer.on_received_message(p0.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p0.clone()));

        // detect loss for 1
        let (recv, rtx) = buffer.on_received_message(p2.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 1);
        assert_eq!(rtx[0], 1);

        // detect loss for 3 and 4
        let (recv, rtx) = buffer.on_received_message(p5.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 2);
        assert_eq!(rtx[0], 3);
        assert_eq!(rtx[1], 4);

        // dup 2 return nothing
        let (recv, rtx) = buffer.on_received_message(p2.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 0);

        // add 3 to the buffer
        let (recv, rtx) = buffer.on_received_message(p3.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 0);

        // add 4 to the buffer
        let (recv, rtx) = buffer.on_received_message(p4.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 0);

        // release 1, 2, 3, 4, 5
        let (recv, rtx) = buffer.on_received_message(p1.clone());
        assert_eq!(recv.len(), 5);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p1.clone()));
        assert_eq!(recv[1], Some(p2.clone()));
        assert_eq!(recv[2], Some(p3.clone()));
        assert_eq!(recv[3], Some(p4.clone()));
        assert_eq!(recv[4], Some(p5.clone()));

        // insertion order 0, 2, 4, loss(1), 5, loss(3)
        let mut buffer = ReceiverBuffer::default();

        // release 0
        let (recv, rtx) = buffer.on_received_message(p0.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p0.clone()));

        // detect loss for 1
        let (recv, rtx) = buffer.on_received_message(p2.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 1);
        assert_eq!(rtx[0], 1);

        // detect loss for 3
        let (recv, rtx) = buffer.on_received_message(p4.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 1);
        assert_eq!(rtx[0], 3);

        // 1 is lost, return up to 2
        let recv = buffer.on_lost_message(1);
        assert_eq!(recv.len(), 2);
        assert_eq!(recv[0], None);
        assert_eq!(recv[1], Some(p2.clone()));

        // 5 is lost
        let recv = buffer.on_lost_message(5);
        assert_eq!(recv.len(), 0);

        // add 3, return up to 5
        let (recv, rtx) = buffer.on_received_message(p3.clone());
        assert_eq!(recv.len(), 3);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p3.clone()));
        assert_eq!(recv[1], Some(p4.clone()));
        assert_eq!(recv[2], None);

        // insertion order 0, beacon(2), beacon(1), 2, 1, 4, beacon(3), 3, 5
        let mut buffer = ReceiverBuffer::default();

        let (recv, rtx) = buffer.on_received_message(p0.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p0.clone()));

        let rtx = buffer.on_beacon_message(2);
        assert_eq!(rtx.len(), 2);
        assert_eq!(rtx[0], 1);
        assert_eq!(rtx[1], 2);

        let rtx = buffer.on_beacon_message(1);
        assert_eq!(rtx.len(), 0);

        let (recv, rtx) = buffer.on_received_message(p2.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 0);

        let (recv, rtx) = buffer.on_received_message(p1.clone());
        assert_eq!(recv.len(), 2);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p1.clone()));
        assert_eq!(recv[1], Some(p2.clone()));

        let (recv, rtx) = buffer.on_received_message(p4.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 1);
        assert_eq!(rtx[0], 3);

        let rtx = buffer.on_beacon_message(3);
        assert_eq!(rtx.len(), 0);

        let (recv, rtx) = buffer.on_received_message(p3.clone());
        assert_eq!(recv.len(), 2);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p3.clone()));
        assert_eq!(recv[1], Some(p4.clone()));

        let (recv, rtx) = buffer.on_received_message(p5.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p5.clone()));

        // test message_already_received function
        // Case 1: No message received yet
        let buffer = ReceiverBuffer::default();
        assert!(!buffer.message_already_received(0));
        assert!(!buffer.message_already_received(1));

        // Case 2: Receive message 0
        let mut buffer = ReceiverBuffer::default();
        let (_recv, _rtx) = buffer.on_received_message(p0.clone());
        // 0 delivered, so 0 is already received, 1 is not
        assert!(buffer.message_already_received(0));
        assert!(!buffer.message_already_received(1));
        assert!(!buffer.message_already_received(2));

        // Case 3: Receive messages 1 and 2 in order
        let (_recv, _rtx) = buffer.on_received_message(p1.clone());
        let (_recv, _rtx) = buffer.on_received_message(p2.clone());
        // 0,1,2 delivered
        assert!(buffer.message_already_received(0));
        assert!(buffer.message_already_received(1));
        assert!(buffer.message_already_received(2));
        assert!(!buffer.message_already_received(3));

        // Case 4: Out-of-order message (skip 3, receive 4)
        let mut buffer = ReceiverBuffer::default();
        let (_recv, _rtx) = buffer.on_received_message(p0.clone());
        let (_recv, _rtx) = buffer.on_received_message(p2.clone());
        let (_recv, _rtx) = buffer.on_received_message(p4.clone());
        // 0 delivered, 1 and 3 missing, 2 and 4 received OOO
        assert!(buffer.message_already_received(0));
        assert!(!buffer.message_already_received(1));
        assert!(buffer.message_already_received(2)); // in buffer
        assert!(!buffer.message_already_received(3));
        assert!(buffer.message_already_received(4)); // in buffer
        assert!(!buffer.message_already_received(5));

        // Case 5: Mark 1 as lost
        let _ = buffer.on_lost_message(1);
        // Now 1 should be marked as received
        assert!(buffer.message_already_received(1));
        // 3 is still missing
        assert!(!buffer.message_already_received(3));
    }

    #[test]
    #[traced_test]
    fn test_receiver_buffer_wraparound() {
        // Test wraparound behavior at MAX_PUBLISH_ID
        let src = Name::from_strings(["org", "ns", "type"]).with_id(0);
        let src_id = src.to_string();
        let name_type = Name::from_strings(["org", "ns", "type"]).with_id(1);

        let slim_header = SlimHeader::new(&src, &name_type, &src_id, None);

        // Create messages near MAX_PUBLISH_ID and wrapping around to 0
        let id_max_minus_1 = MAX_PUBLISH_ID - 1;
        let id_max = MAX_PUBLISH_ID;
        let id_zero = 0;
        let id_one = 1;
        let id_two = 2;

        let h_max_minus_1 = SessionHeader::new(
            ProtoSessionType::PointToPoint.into(),
            ProtoSessionMessageType::Msg.into(),
            0,
            id_max_minus_1,
        );
        let h_max = SessionHeader::new(
            ProtoSessionType::PointToPoint.into(),
            ProtoSessionMessageType::Msg.into(),
            0,
            id_max,
        );
        let h_zero = SessionHeader::new(
            ProtoSessionType::PointToPoint.into(),
            ProtoSessionMessageType::Msg.into(),
            0,
            id_zero,
        );
        let h_one = SessionHeader::new(
            ProtoSessionType::PointToPoint.into(),
            ProtoSessionMessageType::Msg.into(),
            0,
            id_one,
        );
        let h_two = SessionHeader::new(
            ProtoSessionType::PointToPoint.into(),
            ProtoSessionMessageType::Msg.into(),
            0,
            id_two,
        );

        let p_max_minus_1 = Message::builder()
            .with_slim_header(slim_header.clone())
            .with_session_header(h_max_minus_1)
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        let p_max = Message::builder()
            .with_slim_header(slim_header.clone())
            .with_session_header(h_max)
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        let p_zero = Message::builder()
            .with_slim_header(slim_header.clone())
            .with_session_header(h_zero)
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        let p_one = Message::builder()
            .with_slim_header(slim_header.clone())
            .with_session_header(h_one)
            .application_payload("", vec![])
            .build_publish()
            .unwrap();
        let p_two = Message::builder()
            .with_slim_header(slim_header.clone())
            .with_session_header(h_two)
            .application_payload("", vec![])
            .build_publish()
            .unwrap();

        // Test Case 1: Sequential messages across wraparound
        let mut buffer = ReceiverBuffer::default();

        // Receive MAX-1
        let (recv, rtx) = buffer.on_received_message(p_max_minus_1.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p_max_minus_1.clone()));

        // Receive MAX
        let (recv, rtx) = buffer.on_received_message(p_max.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p_max.clone()));

        // Receive 0 (wraparound)
        let (recv, rtx) = buffer.on_received_message(p_zero.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p_zero.clone()));

        // Receive 1
        let (recv, rtx) = buffer.on_received_message(p_one.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p_one.clone()));

        // Test Case 2: Out-of-order across wraparound
        let mut buffer = ReceiverBuffer::default();

        // Start with MAX-1
        let (recv, rtx) = buffer.on_received_message(p_max_minus_1.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);

        // Receive 1 (skipping MAX and 0) - should detect loss
        let (recv, rtx) = buffer.on_received_message(p_one.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 2);
        assert_eq!(rtx[0], id_max);
        assert_eq!(rtx[1], id_zero);

        // Now receive MAX
        let (recv, rtx) = buffer.on_received_message(p_max.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p_max.clone()));

        // Now receive 0 - should release both 0 and 1
        let (recv, rtx) = buffer.on_received_message(p_zero.clone());
        assert_eq!(recv.len(), 2);
        assert_eq!(rtx.len(), 0);
        assert_eq!(recv[0], Some(p_zero.clone()));
        assert_eq!(recv[1], Some(p_one.clone()));

        // Test Case 3: Large gap across wraparound
        let mut buffer = ReceiverBuffer::default();

        // Start with MAX-1
        let (recv, rtx) = buffer.on_received_message(p_max_minus_1.clone());
        assert_eq!(recv.len(), 1);
        assert_eq!(rtx.len(), 0);

        // Skip to 2 (missing MAX, 0, 1)
        let (recv, rtx) = buffer.on_received_message(p_two.clone());
        assert_eq!(recv.len(), 0);
        assert_eq!(rtx.len(), 3);
        assert_eq!(rtx[0], id_max);
        assert_eq!(rtx[1], id_zero);
        assert_eq!(rtx[2], id_one);

        // Test message_already_received with wraparound
        let mut buffer = ReceiverBuffer::default();
        let (_recv, _rtx) = buffer.on_received_message(p_max_minus_1.clone());
        let (_recv, _rtx) = buffer.on_received_message(p_max.clone());
        let (_recv, _rtx) = buffer.on_received_message(p_zero.clone());

        // MAX-1, MAX, and 0 should be marked as received
        assert!(buffer.message_already_received(id_max_minus_1 as usize));
        assert!(buffer.message_already_received(id_max as usize));
        assert!(buffer.message_already_received(id_zero as usize));
        // 1 should not be received yet
        assert!(!buffer.message_already_received(id_one as usize));
    }
}
