// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use crate::session::{Id, Error, Session, State, SessionType, Common};

use agp_datapath::pubsub::proto::pubsub::v1::Message;

struct FireAndForget {
    common: Common,
}

impl FireAndForget {
    fn new(id: Id, session_type: SessionType) -> FireAndForget {
        FireAndForget {
            common: Common::new(id, session_type),
        }
    }
}

impl Session for FireAndForget {
    fn id(&self) -> u64 {
        0
    }

    fn state(&self) -> &State {
        self.common.state()
    }

    fn session_type(&self) -> &SessionType {
        self.common.session_type()
    }

    fn on_publish(&self, _message: &mut Message) -> Result<(), Error> {
        Ok(())
    }

    fn on_receive(&self, _message: &mut Message) -> Result<(), Error> {
        Ok(())
    }
}