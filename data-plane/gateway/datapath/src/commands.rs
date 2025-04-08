// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use tokio::sync::oneshot;

#[derive(Debug)]
pub enum ControlCommand {
    Subscribe {
        reply: oneshot::Sender<Result<(), String>>,
    },
    Unsubscribe {
        reply: oneshot::Sender<Result<(), String>>,
    },
}
