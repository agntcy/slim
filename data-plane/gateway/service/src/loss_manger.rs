// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use agp_datapath::pubsub::proto::pubsub::v1::Message;

struct LossManager {
    // ID of the last packet sent to the application
    // The IDs are sequential and starts from 1
    last_sent: u32,
    // Msg ID of the first valid entry in the buffer
    base_id: u32,
    // First valid entry in the buffer. Packets may be
    // removed from the front of the buffer and we want to 
    // avoid copies. This pointer keeps track of the valid entries
    first_entry: u32,
    // Buffer of valid messages received Out-of-Oreder (OOO)
    // waiting to be delivered to the application
    buffer: Vec<Message>
}

impl Default for LossManager {
    fn default() -> Self {
        LossManager {
            last_sent: 0,
            base_id: 0,
            first_entry: 0, 
            buffer: vec![],
        }
    }
}

impl LossManager {
    // TODO add trigger for RTX
    fn received_message(&mut self, msg: Message) -> Result<Vec<Option<Message>>, Error> {
        if let Some(pubmsg) =  get_message_as_publish(msg) {
            let msg_id = match get_msg_id(publish) {
                Err(_) => {
                    Ok(()); // TODO return an error
                }
                Ok(msg_id) => msg_id,
            };

            // no loss detected, return message
            if msg_id = self.last_sent + 1 {
                self.last_sent = msg_id;
                return Ok(vec![Some(msg)]);
            }

            if self.buffer.is_empty() {
                // this is an OOO message
                // if msg_id <= last_sent drop the message
                // otherwise put it in the buffer and return an empty vector
                if  msg_id <= self.last_sent {
                    // TODO return an error
                    Ok(())
                }
                
                // the min buffer size is msg_id - last_sent;
                self.base_id = self.last_sent + 1;
                self.first_entry = 0;
                self.buffer = vec![None, (msg_id - self.last_sent) - 1];
                self.buffer.push(msg);
                return Ok(vec![])
            } else {
                // there is already something in the buffer
                // if msg_id < base_id drop the message
                if  msg_id < self.base_id {
                    // TODO return an error
                    Ok(())
                }

                // if base_id <= msg_id < base_id + buffer_len()
                // add message to the buffer
                // check if something can be released
                if mgs_id < (self.base_id + self.buffer.len()) {
                    self.buffer[msg_id - self.base_id + self.first_entry] = msg; // double check
                    let mut return_vec = vec![];
                    let mut i = self.first_entry;
                    while i < self.buffer.len() {
                        if self.buffer[i].is_some() {
                            return_vec.push(self.buffer[i]);
                            self.base_id += 1;
                            self.first_entry += 1;
                            self.last_sent += 1; 
                        }
                        i += 1; 
                    }

                    // if self.first_entry == self.buffer.len() reset buffer
                    if elf.first_entry == self.buffer.len() {
                        self.buffer = vec![];
                        self.base_id = 0;
                        self.first_entry = 0;
                    }   
                    Ok(return_vec)
                }

                // msg_id < buffer_base + buffer_len()
                // keep the message in the buffer for later
                let mut i = self.buffer_base + self.buffer.len();
                while i < msg_id {
                    self.buffer.push(None);
                    i += 1;
                }
                self.buffer.push(msg);
                Ok(vec![])
            }
        }
    }
}