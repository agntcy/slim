// SPDX-FileCopyrightText: Copyright (c) 2025 Cisco and/or its affiliates.
// SPDX-License-Identifier: Apache-2.0

use crate::session::{Common, Error, Id, Session, SessionType, State};

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

    fn on_message_from_gateway(
        &self,
        message: Message,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send>> {
        // clone tx
        let tx = self.common.north_tx();

        Box::pin(async move {
            // Nothing to do here, just pass the message to the app
            tx.send(message)
                .await
                .map_err(|e| Error::AppTransmissionError(e.to_string()))
        })
    }

    fn on_message_from_app(
        &self,
        message: Message,
    ) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), Error>> + Send>> {
        // clone tx
        let tx = self.common.south_tx();

        Box::pin(async move {
            // Nothing to do here, just pass the message to the app
            tx.send(message)
                .await
                .map_err(|e| Error::AppTransmissionError(e.to_string()))
        })
    }
}
