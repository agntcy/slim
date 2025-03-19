// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use crate::session::{Common, Error, Id, MessageDirection, Session, SessionDirection, State};

use agp_datapath::messages::utils;
use agp_datapath::pubsub::proto::pubsub::v1::Message;
use agp_datapath::pubsub::proto::pubsub::v1::ServiceHeaderType;

pub(crate) struct FireAndForget {
    common: Common,
}

impl FireAndForget {
    pub(crate) fn new(id: Id, session_type: SessionDirection) -> FireAndForget {
        FireAndForget {
            common: Common::new(id, session_type),
        }
    }
}

impl Session for FireAndForget {
    fn id(&self) -> Id {
        0
    }

    fn state(&self) -> &State {
        self.common.state()
    }

    fn session_type(&self) -> &SessionDirection {
        self.common.session_type()
    }

    fn on_message(
        &self,
        mut message: Message,
        direction: MessageDirection,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send>> {
        // clone tx
        let tx = match direction {
            MessageDirection::North => self.common.north_tx().clone(),
            MessageDirection::South => self.common.south_tx().clone(),
        };

        // set the session type
        let header = utils::get_session_header_as_mut(&mut message);
        if header.is_none() {
            return Box::pin(async move {
                Err(Error::AppTransmissionError("missing header".to_string()))
            });
        }

        header.unwrap().header_type = utils::service_type_to_int(ServiceHeaderType::CtrlFnf);

        Box::pin(async move {
            // Nothing to do here, just pass the message
            tx.send(message)
                .await
                .map_err(|e| Error::AppTransmissionError(e.to_string()))
        })
    }
}
