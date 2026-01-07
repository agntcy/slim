// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::sync::Arc;

use slim_datapath::api::{
    ProtoMessage as DataPlaneMessage, ProtoSessionMessageType, ProtoSessionType,
};
use slim_datapath::messages::Name;
use slim_session::timer::{Timer, TimerObserver};
use slim_session::timer_factory::TimerSettings;
use tokio::sync::mpsc::Sender;
use tonic::{Status, async_trait};
use tracing::debug;

/// Timer observer for controller that sends to the data plane channel
struct ControllerTimerObserver {
    tx: Sender<Result<DataPlaneMessage, Status>>,
    message_type: ProtoSessionMessageType,
    name: Name,
}

#[async_trait]
impl TimerObserver for ControllerTimerObserver {
    async fn on_timeout(&self, message_id: u32, timeouts: u32) {
        debug!(timer_id = %message_id, %timeouts, "timer timeout in controller");

        match DataPlaneMessage::builder()
            .source(self.name.clone())
            .destination(self.name.clone())
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(self.message_type)
            .message_id(message_id)
            .build_publish()
        {
            Ok(timeout_msg) => {
                if let Err(e) = self.tx.send(Ok(timeout_msg)).await {
                    debug!("Failed to send controller timeout message: {}", e);
                }
            }
            Err(e) => {
                debug!("Failed to build controller timeout message: {}", e);
            }
        }
    }

    async fn on_failure(&self, message_id: u32, timeouts: u32) {
        debug!(timer_id = %message_id, %timeouts, "timer failure in controller");

        match DataPlaneMessage::builder()
            .source(self.name.clone())
            .destination(self.name.clone())
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(self.message_type)
            .message_id(message_id)
            .build_publish()
        {
            Ok(failure_msg) => {
                if let Err(e) = self.tx.send(Ok(failure_msg)).await {
                    debug!("Failed to send controller timer failure message: {}", e);
                }
            }
            Err(e) => {
                debug!("Failed to build controller timer failure message: {}", e);
            }
        }
    }

    async fn on_stop(&self, message_id: u32) {
        debug!(timer_id = %message_id, "timer stopped");
        match DataPlaneMessage::builder()
            .source(self.name.clone())
            .destination(self.name.clone())
            .session_type(ProtoSessionType::Multicast)
            .session_message_type(self.message_type)
            .message_id(message_id)
            .build_publish()
        {
            Ok(failure_msg) => {
                if let Err(e) = self.tx.send(Ok(failure_msg)).await {
                    debug!("Failed to send controller timer stopped message: {}", e);
                }
            }
            Err(e) => {
                debug!("Failed to build controller timer stopped message: {}", e);
            }
        }
    }
}

pub struct ControllerTimerFactory {
    tx: Sender<Result<DataPlaneMessage, Status>>,
    settings: TimerSettings,
}

impl ControllerTimerFactory {
    pub fn new(settings: TimerSettings, tx: Sender<Result<DataPlaneMessage, Status>>) -> Self {
        Self { tx, settings }
    }

    pub fn create_timer(&self, id: u32) -> Timer {
        Timer::new(
            id,
            self.settings.timer_type.clone(),
            self.settings.duration,
            self.settings.max_duration,
            self.settings.max_retries,
        )
    }

    pub fn create_and_start_timer(
        &self,
        id: u32,
        message_type: ProtoSessionMessageType,
        name: Name,
    ) -> Timer {
        let timer = self.create_timer(id);
        self.start_timer(&timer, message_type, name);
        timer
    }

    pub fn start_timer(&self, timer: &Timer, message_type: ProtoSessionMessageType, name: Name) {
        let observer = ControllerTimerObserver {
            tx: self.tx.clone(),
            message_type,
            name,
        };
        timer.start(Arc::new(observer));
    }
}
