// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::{sync::Arc, time::Duration};

use slim_datapath::{api::ProtoSessionMessageType, messages::Name};
use tokio::sync::mpsc::Sender;
use tonic::async_trait;
use tracing::debug;

use crate::{
    common::SessionMessage,
    timer::{Timer, TimerObserver, TimerType},
};

struct ReliableTimerObserver {
    tx: Sender<SessionMessage>,
    message_type: ProtoSessionMessageType,
    name: Option<Name>,
}

#[async_trait]
impl TimerObserver for ReliableTimerObserver {
    async fn on_timeout(&self, message_id: u32, timeouts: u32) {
        self.tx
            .send(SessionMessage::TimerTimeout {
                message_id,
                message_type: self.message_type,
                name: self.name.clone(),
                timeouts,
            })
            .await
            .expect("failed to send timer timeout");
    }

    async fn on_failure(&self, message_id: u32, timeouts: u32) {
        // remove the state for the lost message
        self.tx
            .send(SessionMessage::TimerFailure {
                message_id,
                message_type: self.message_type,
                name: self.name.clone(),
                timeouts,
            })
            .await
            .expect("failed to send timer failure");
    }

    async fn on_stop(&self, message_id: u32) {
        debug!("timer stopped: {}", message_id);
    }
}

#[derive(Clone)]
pub struct TimerSettings {
    pub duration: Duration,
    pub max_duration: Option<Duration>,
    pub max_retries: Option<u32>,
    pub timer_type: TimerType,
}

impl TimerSettings {
    /// Create a new TimerSettings with the specified parameters
    pub fn new(
        duration: Duration,
        max_duration: Option<Duration>,
        max_retries: Option<u32>,
        timer_type: TimerType,
    ) -> Self {
        Self {
            duration,
            max_duration,
            max_retries,
            timer_type,
        }
    }

    /// Create a constant timer settings with the specified duration
    pub fn constant(duration: Duration) -> Self {
        Self {
            duration,
            max_duration: None,
            max_retries: None,
            timer_type: TimerType::Constant,
        }
    }

    /// Create an exponential timer settings with the specified duration and max duration
    pub fn exponential(initial_duration: Duration, max_duration: Option<Duration>) -> Self {
        Self {
            duration: initial_duration,
            max_duration,
            max_retries: None,
            timer_type: TimerType::Exponential,
        }
    }

    /// Set the maximum number of retries before failure
    pub fn with_max_retries(mut self, max_retries: u32) -> Self {
        self.max_retries = Some(max_retries);
        self
    }
}

pub struct TimerFactory {
    //observer: Arc<ReliableTimerObserver>,
    tx: Sender<SessionMessage>,
    settings: TimerSettings,
}

impl TimerFactory {
    pub fn new(settings: TimerSettings, tx: Sender<SessionMessage>) -> Self {
        Self {
            tx: tx.clone(),
            settings,
        }
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
        name: Option<Name>,
    ) -> Timer {
        let t = Timer::new(
            id,
            self.settings.timer_type.clone(),
            self.settings.duration,
            self.settings.max_duration,
            self.settings.max_retries,
        );
        self.start_timer(&t, message_type, name);
        t
    }

    pub fn start_timer(
        &self,
        timer: &Timer,
        message_type: ProtoSessionMessageType,
        name: Option<Name>,
    ) {
        // start timer
        let observer = ReliableTimerObserver {
            tx: self.tx.clone(),
            message_type,
            name,
        };
        timer.start(Arc::new(observer));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use tokio::time::timeout;

    // Helper function to create test names
    fn test_name() -> Name {
        Name::from_strings(["test", "org", "app"]).with_id(1)
    }

    #[tokio::test]
    async fn test_timer_factory_new() {
        // Arrange
        let (tx, _rx) = mpsc::channel(10);
        let settings =
            TimerSettings::new(Duration::from_millis(100), None, None, TimerType::Constant);

        // Act
        let factory = TimerFactory::new(settings, tx);

        // Assert
        // Just check that the factory was created successfully
        assert_eq!(factory.settings.duration, Duration::from_millis(100));
        assert!(factory.settings.max_duration.is_none());
        assert!(factory.settings.max_retries.is_none());
        matches!(factory.settings.timer_type, TimerType::Constant);
    }

    #[tokio::test]
    async fn test_create_and_start_timer() {
        // Arrange
        let (tx, mut rx) = mpsc::channel(10);
        let settings = TimerSettings::new(
            Duration::from_millis(50),
            None,
            Some(1), // Only 1 retry to make test faster
            TimerType::Constant,
        );
        let factory = TimerFactory::new(settings, tx);
        let timer_id = 123;
        let name = test_name();

        let _timer = factory.create_and_start_timer(
            timer_id,
            ProtoSessionMessageType::DiscoveryRequest,
            Some(name.clone()),
        );

        // Assert - we should receive a timeout message
        let message = timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("Should receive a message within timeout")
            .expect("Should receive a message");

        match message {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name: received_name,
                timeouts,
            } => {
                assert_eq!(message_id, timer_id);
                assert_eq!(message_type, ProtoSessionMessageType::DiscoveryRequest);
                assert_eq!(received_name, Some(name));
                assert_eq!(timeouts, 1);
            }
            _ => panic!("Expected TimerTimeout message"),
        }
    }

    #[tokio::test]
    async fn test_timer_timeout_with_constant_timer() {
        // Arrange
        let (tx, mut rx) = mpsc::channel(10);
        let settings = TimerSettings::new(
            Duration::from_millis(30),
            None,
            Some(2), // Allow 2 retries
            TimerType::Constant,
        );
        let factory = TimerFactory::new(settings, tx);
        let timer_id = 456;
        let name = test_name();

        // Act
        let timer = factory.create_timer(timer_id);
        factory.start_timer(
            &timer,
            ProtoSessionMessageType::DiscoveryRequest,
            Some(name.clone()),
        );

        // Assert - we should receive multiple timeout messages
        let first_timeout = timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("Should receive first timeout")
            .expect("Should receive a message");

        match first_timeout {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name: received_name,
                timeouts,
            } => {
                assert_eq!(message_id, timer_id);
                assert_eq!(message_type, ProtoSessionMessageType::DiscoveryRequest);
                assert_eq!(timeouts, 1);
                assert_eq!(received_name, Some(name.clone()));
            }
            _ => panic!("Expected TimerTimeout message for first timeout"),
        }

        let second_timeout = timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("Should receive second timeout")
            .expect("Should receive a message");

        match second_timeout {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name: received_name,
                timeouts,
            } => {
                assert_eq!(message_id, timer_id);
                assert_eq!(message_type, ProtoSessionMessageType::DiscoveryRequest);
                assert_eq!(timeouts, 2);
                assert_eq!(received_name, Some(name.clone()));
            }
            _ => panic!("Expected TimerTimeout message for second timeout"),
        }
    }

    #[tokio::test]
    async fn test_timer_failure_after_max_retries() {
        // Arrange
        let (tx, mut rx) = mpsc::channel(10);
        let settings = TimerSettings::new(
            Duration::from_millis(30),
            None,
            Some(1), // Only 1 retry, then failure
            TimerType::Constant,
        );
        let factory = TimerFactory::new(settings, tx);
        let timer_id = 789;
        let name = test_name();

        // Act
        let timer = factory.create_timer(timer_id);
        factory.start_timer(
            &timer,
            ProtoSessionMessageType::DiscoveryRequest,
            Some(name.clone()),
        );

        // Assert - we should receive a timeout followed by a failure
        let timeout_message = timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("Should receive timeout message")
            .expect("Should receive a message");

        match timeout_message {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name: received_name,
                timeouts,
            } => {
                assert_eq!(message_id, timer_id);
                assert_eq!(message_type, ProtoSessionMessageType::DiscoveryRequest);
                assert_eq!(timeouts, 1);
                assert_eq!(received_name, Some(name.clone()));
            }
            _ => panic!("Expected TimerTimeout message in failure test"),
        }

        let failure_message = timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("Should receive failure message")
            .expect("Should receive a message");

        match failure_message {
            SessionMessage::TimerFailure {
                message_id,
                message_type,
                name: received_name,
                timeouts,
            } => {
                assert_eq!(message_id, timer_id);
                assert_eq!(message_type, ProtoSessionMessageType::DiscoveryRequest);
                assert_eq!(timeouts, 2);
                assert_eq!(received_name, Some(name.clone()));
            }
            _ => panic!("Expected TimerFailure message"),
        }
    }

    #[tokio::test]
    async fn test_exponential_timer() {
        // Arrange
        let (tx, mut rx) = mpsc::channel(10);
        let settings = TimerSettings::new(
            Duration::from_millis(20),        // Start with 20ms
            Some(Duration::from_millis(100)), // Max 100ms
            Some(2),                          // Allow 2 retries before failure
            TimerType::Exponential,
        );
        let factory = TimerFactory::new(settings, tx);
        let timer_id = 999;
        let name = test_name();

        // Act
        let timer = factory.create_timer(timer_id);
        factory.start_timer(
            &timer,
            ProtoSessionMessageType::DiscoveryRequest,
            Some(name.clone()),
        );

        // Assert - we should receive timeouts with exponentially increasing intervals
        let first_timeout = timeout(Duration::from_millis(150), rx.recv())
            .await
            .expect("Should receive first timeout")
            .expect("Should receive a message");

        match first_timeout {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name: received_name,
                timeouts,
            } => {
                assert_eq!(message_id, timer_id);
                assert_eq!(message_type, ProtoSessionMessageType::DiscoveryRequest);
                assert_eq!(timeouts, 1);
                assert_eq!(received_name, Some(name.clone()));
            }
            _ => panic!("Expected TimerTimeout message for exponential timer first timeout"),
        }

        let second_timeout = timeout(Duration::from_millis(200), rx.recv())
            .await
            .expect("Should receive second timeout")
            .expect("Should receive a message");

        match second_timeout {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                name: received_name,
                timeouts,
            } => {
                assert_eq!(message_id, timer_id);
                assert_eq!(message_type, ProtoSessionMessageType::DiscoveryRequest);
                assert_eq!(timeouts, 2);
                assert_eq!(received_name, Some(name.clone()));
            }
            _ => panic!("Expected TimerTimeout message for exponential timer second timeout"),
        }
    }

    #[tokio::test]
    async fn test_timer_settings_with_all_options() {
        // Arrange
        let (tx, _rx) = mpsc::channel(10);
        let duration = Duration::from_millis(500);
        let max_duration = Some(Duration::from_secs(5));
        let max_retries = Some(10);
        let timer_type = TimerType::Exponential;

        let settings = TimerSettings::new(duration, max_duration, max_retries, timer_type);

        // Act
        let factory = TimerFactory::new(settings, tx);

        // Assert
        assert_eq!(factory.settings.duration, duration);
        assert_eq!(factory.settings.max_duration, max_duration);
        assert_eq!(factory.settings.max_retries, max_retries);
        matches!(factory.settings.timer_type, TimerType::Exponential);
    }

    #[tokio::test]
    async fn test_multiple_timers() {
        // Arrange
        let (tx, mut rx) = mpsc::channel(20);
        let settings = TimerSettings::new(
            Duration::from_millis(50),
            None,
            Some(1),
            TimerType::Constant,
        );
        let factory = TimerFactory::new(settings, tx);
        let name1 = Name::from_strings(["test", "org", "app1"]).with_id(1);
        let name2 = Name::from_strings(["test", "org", "app2"]).with_id(2);

        // Act - create and start multiple timers
        let timer1 = factory.create_and_start_timer(
            100,
            ProtoSessionMessageType::DiscoveryRequest,
            Some(name1.clone()),
        );
        let timer2 = factory.create_and_start_timer(
            200,
            ProtoSessionMessageType::DiscoveryRequest,
            Some(name2.clone()),
        );

        // Assert - we should receive messages from both timers
        let mut received_ids = Vec::new();

        for _ in 0..2 {
            let message = timeout(Duration::from_millis(200), rx.recv())
                .await
                .expect("Should receive a message within timeout")
                .expect("Should receive a message");

            match message {
                SessionMessage::TimerTimeout {
                    message_id,
                    message_type: _,
                    timeouts,
                    name: _,
                } => {
                    received_ids.push(message_id);
                    assert_eq!(timeouts, 1);
                }
                _ => panic!("Expected TimerTimeout message in multiple timers test"),
            }
        }

        received_ids.sort();
        assert_eq!(received_ids, vec![100, 200]);

        // Clean up timers to avoid them running indefinitely
        drop(timer1);
        drop(timer2);
    }

    #[test]
    fn test_timer_settings_creation() {
        // Test creating TimerSettings with different configurations
        let settings1 =
            TimerSettings::new(Duration::from_millis(100), None, None, TimerType::Constant);

        assert_eq!(settings1.duration, Duration::from_millis(100));
        assert!(settings1.max_duration.is_none());
        assert!(settings1.max_retries.is_none());
        matches!(settings1.timer_type, TimerType::Constant);

        let settings2 = TimerSettings::new(
            Duration::from_secs(1),
            Some(Duration::from_secs(10)),
            Some(5),
            TimerType::Exponential,
        );

        assert_eq!(settings2.duration, Duration::from_secs(1));
        assert_eq!(settings2.max_duration, Some(Duration::from_secs(10)));
        assert_eq!(settings2.max_retries, Some(5));
        matches!(settings2.timer_type, TimerType::Exponential);
    }

    #[test]
    fn test_timer_settings_convenience_constructors() {
        // Test constant timer constructor
        let constant_settings = TimerSettings::constant(Duration::from_millis(500));
        assert_eq!(constant_settings.duration, Duration::from_millis(500));
        assert!(constant_settings.max_duration.is_none());
        assert!(constant_settings.max_retries.is_none());
        matches!(constant_settings.timer_type, TimerType::Constant);

        // Test exponential timer constructor
        let exponential_settings =
            TimerSettings::exponential(Duration::from_millis(100), Some(Duration::from_secs(5)));
        assert_eq!(exponential_settings.duration, Duration::from_millis(100));
        assert_eq!(
            exponential_settings.max_duration,
            Some(Duration::from_secs(5))
        );
        assert!(exponential_settings.max_retries.is_none());
        matches!(exponential_settings.timer_type, TimerType::Exponential);

        // Test fluent builder pattern
        let settings_with_retries =
            TimerSettings::constant(Duration::from_millis(250)).with_max_retries(10);
        assert_eq!(settings_with_retries.duration, Duration::from_millis(250));
        assert_eq!(settings_with_retries.max_retries, Some(10));
        matches!(settings_with_retries.timer_type, TimerType::Constant);
    }

    #[tokio::test]
    async fn test_timer_factory_with_convenience_constructors() {
        // Arrange
        let (tx, mut rx) = mpsc::channel(10);
        let settings = TimerSettings::constant(Duration::from_millis(40)).with_max_retries(1);
        let factory = TimerFactory::new(settings, tx);
        let timer_id = 888;
        let name = test_name();

        // Act
        let _timer = factory.create_and_start_timer(
            timer_id,
            ProtoSessionMessageType::DiscoveryRequest,
            Some(name.clone()),
        );

        // Assert
        let timeout_message = timeout(Duration::from_millis(100), rx.recv())
            .await
            .expect("Should receive timeout message")
            .expect("Should receive a message");

        match timeout_message {
            SessionMessage::TimerTimeout {
                message_id,
                message_type,
                timeouts,
                name: received_name,
            } => {
                assert_eq!(message_id, timer_id);
                assert_eq!(message_type, ProtoSessionMessageType::DiscoveryRequest);
                assert_eq!(timeouts, 1);
                assert_eq!(received_name, Some(name.clone()));
            }
            _ => panic!("Expected TimerTimeout message with convenience constructors"),
        }
    }
}
