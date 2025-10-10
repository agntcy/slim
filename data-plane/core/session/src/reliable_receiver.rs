// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

/*struct ReliableReceiver {
    // buffer with received packets one per participant
    buffer: HashMap<Name,ReceiverBuffer>, 
    // list of pending RTX requests per name/id
    pending_rtxs: HashMap<Name, HashMap<u32, (timer::Timer, Message)>>,
    timer_settings: TimerSettings,
    is_sequental: bool, //use the buffer and rxt or not
    tx: T, // send to slim
    send_to_app: bool,
}

impl ReliableReceiver {
    fn on_message_from_slim() {
        // if sequential
        //      create a new buffer if needed
        //      check for RTX and send them
        // send ack back
        // send to the application if true
    }

    fn on_rm_participant()  {
        // remove all the state for this participant
    }
}*/ 