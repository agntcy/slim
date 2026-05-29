// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Standard library imports
use std::sync::Arc;

// Third-party crates
use tokio::time::{self, Duration};
use tokio_util::sync::CancellationToken;
use tonic::async_trait;
use tracing::trace;

#[async_trait]
pub trait TimerObserver {
    async fn on_timeout(&self, timer_id: u32, timeouts: u32);
    async fn on_failure(&self, timer_id: u32, timeouts: u32);
    async fn on_stop(&self, timer_id: u32);
}

#[derive(Debug, Clone)]
pub enum TimerType {
    Constant = 0,
    Exponential = 1,
}

#[derive(Debug)]
pub struct Timer {
    /// timer id
    timer_id: u32,

    /// timer type
    timer_type: TimerType,

    /// constant timer: timer duration
    /// exponential timer: min timer duration. at every new timer the duration is computers as last_duration * 2
    duration: Duration,

    /// constant timer: None
    /// exponential timer: maximum timer duration. once the duration reaches this time it will not be encreased anymore
    max_duration: Option<Duration>,

    /// if not None, it indicates the maximum number of retryes before call on_failure
    /// if set to None the timer will go on forever unless cancelled
    max_retries: Option<u32>,

    /// token used to cancel the timer
    cancellation_token: CancellationToken,
}

impl Timer {
    pub fn new(
        timer_id: u32,
        timer_type: TimerType,
        duration: Duration,
        max_duration: Option<Duration>,
        max_retries: Option<u32>,
    ) -> Self {
        Timer {
            timer_id,
            timer_type,
            duration,
            max_duration,
            max_retries,
            cancellation_token: CancellationToken::new(),
        }
    }

    pub fn start<T: TimerObserver + Send + Sync + 'static>(&self, observer: Arc<T>) {
        let timer_id = self.timer_id;
        let timer_type = self.timer_type.clone();
        let duration = self.duration;
        let max_retries = self.max_retries;
        let max_duration = self.max_duration;
        let cancellation_token = self.cancellation_token.clone();

        tokio::spawn(async move {
            let mut retry = 0;
            let mut timeouts = 0;
            let mut last_duration = duration;

            trace!(%timer_id, "timer started");
            loop {
                let timer_duration = match timer_type {
                    TimerType::Constant => {
                        trace!(
                            %timer_id, next_ms = duration.as_millis(),
                            "constant timer",
                        );
                        duration
                    }
                    TimerType::Exponential => {
                        let mut d = duration;
                        if timeouts != 0 {
                            d = last_duration * 2;
                        }
                        match max_duration {
                            None => {
                                trace!(
                                    %timer_id, next_ms = d.as_millis(),
                                    "exponential timer",
                                );
                                last_duration = d;
                                d
                            }
                            Some(max_d) => {
                                if d > max_d {
                                    trace!(
                                        %timer_id,
                                        next_ms = max_d.as_millis(),
                                        "exponential timer (use max duration)",
                                    );
                                    last_duration = max_d;
                                    max_d
                                } else {
                                    trace!(
                                        %timer_id, next_ms = max_d.as_millis(),
                                        "exponential timer",
                                    );
                                    last_duration = d;
                                    d
                                }
                            }
                        }
                    }
                };

                let timer = time::sleep(timer_duration);
                tokio::pin!(timer);

                tokio::select! {
                    _ = timer.as_mut() => {
                        timeouts += 1;
                        match max_retries {
                            Some(max) => {
                                if retry < max {
                                    observer.on_timeout(timer_id, timeouts).await
                                } else {
                                    observer.on_failure(timer_id, timeouts).await;
                                    break;
                                }
                            }
                            None => observer.on_timeout(timer_id, timeouts).await
                        }
                        retry += 1;
                    },
                    _ = cancellation_token.cancelled() => {
                        observer.on_stop(timer_id).await;
                        break;
                    },
                }
            }
        });
    }

    pub fn stop(&mut self) {
        self.cancellation_token.cancel();
        self.cancellation_token = CancellationToken::new();
    }

    pub fn reset<T: TimerObserver + Send + Sync + 'static>(&mut self, observer: Arc<T>) {
        self.stop();
        self.start(observer);
    }

    pub fn get_id(&self) -> u32 {
        self.timer_id
    }
}

impl Drop for Timer {
    fn drop(&mut self) {
        self.cancellation_token.cancel();
    }
}

// tests
#[cfg(test)]
mod tests {
    use tracing::debug;
    use tracing_test::traced_test;

    use super::*;

    struct Observer {
        id: u32,
    }

    #[async_trait]
    impl TimerObserver for Observer {
        async fn on_timeout(&self, timer_id: u32, timeouts: u32) {
            debug!(
                %timeouts, %timer_id,
                "timeout occurred, retry",
            );
        }

        async fn on_failure(&self, timer_id: u32, timeouts: u32) {
            debug!(
                %timeouts, %timer_id,
                "timeout occurred, stop retry",
            );
        }

        async fn on_stop(&self, timer_id: u32) {
            debug!(%timer_id, "timer cancelled");
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_timer() {
        let o = Arc::new(Observer { id: 10 });
        let t = Timer::new(
            o.id,
            TimerType::Constant,
            Duration::from_millis(100),
            None,
            Some(3),
        );

        t.start(o);

        time::sleep(Duration::from_millis(500)).await;

        // check logs to validate the test
        let expected_msg = "timeout occurred, retry timeouts=1 timer_id=10";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout occurred, retry timeouts=2 timer_id=10";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout occurred, retry timeouts=3 timer_id=10";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout occurred, stop retry timeouts=4 timer_id=10";
        assert!(logs_contain(expected_msg));

        let o = Arc::new(Observer { id: 20 });
        let t = Timer::new(
            o.id,
            TimerType::Exponential,
            Duration::from_millis(100),
            Some(Duration::from_millis(400)),
            Some(3),
        );

        t.start(o);
        time::sleep(Duration::from_millis(1200)).await;

        let expected_msg = "exponential timer timer_id=20 next_ms=400";
        assert!(logs_contain(expected_msg));
        let expected_msg = "exponential timer timer_id=20 next_ms=400";
        assert!(logs_contain(expected_msg));
        let expected_msg = "exponential timer timer_id=20 next_ms=400";
        assert!(logs_contain(expected_msg));
        let expected_msg = "exponential timer (use max duration) timer_id=20 next_ms=400";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout occurred, stop retry timeouts=4 timer_id=20";
        assert!(logs_contain(expected_msg));

        let o = Arc::new(Observer { id: 30 });
        let mut t = Timer::new(
            o.id,
            TimerType::Exponential,
            Duration::from_millis(100),
            None,
            None,
        );

        t.start(o);

        time::sleep(Duration::from_millis(2000)).await;
        t.stop();
        time::sleep(Duration::from_millis(500)).await;
        let expected_msg = "exponential timer timer_id=30 next_ms=400";
        assert!(logs_contain(expected_msg));
        let expected_msg = "exponential timer timer_id=30 next_ms=800";
        assert!(logs_contain(expected_msg));
        let expected_msg = "exponential timer timer_id=30 next_ms=1600";
        assert!(logs_contain(expected_msg));
        let expected_msg = "exponential timer timer_id=30 next_ms=800";
        assert!(logs_contain(expected_msg));
        let expected_msg = "exponential timer timer_id=30 next_ms=1600";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timer cancelled timer_id=30";
        assert!(logs_contain(expected_msg))
    }

    #[tokio::test]
    #[traced_test]
    async fn test_timer_stop() {
        let o = Arc::new(Observer { id: 10 });

        let mut t = Timer::new(
            o.id,
            TimerType::Constant,
            Duration::from_millis(100),
            None,
            Some(5),
        );

        t.start(o);

        time::sleep(Duration::from_millis(350)).await;

        t.stop();

        time::sleep(Duration::from_millis(500)).await;

        // check logs to validate the test
        let expected_msg = "timeout occurred, retry timeouts=1 timer_id=10";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout occurred, retry timeouts=2 timer_id=10";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout occurred, retry timeouts=3 timer_id=10";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timer cancelled timer_id=10";
        assert!(logs_contain(expected_msg));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_multiple_timers() {
        let o1 = Arc::new(Observer { id: 1 });
        let o2 = Arc::new(Observer { id: 2 });
        let o3 = Arc::new(Observer { id: 3 });

        let mut t1 = Timer::new(
            o1.id,
            TimerType::Constant,
            Duration::from_millis(100),
            None,
            Some(5),
        );
        let mut t2 = Timer::new(
            o2.id,
            TimerType::Constant,
            Duration::from_millis(200),
            None,
            Some(5),
        );
        let mut t3 = Timer::new(
            o3.id,
            TimerType::Constant,
            Duration::from_millis(200),
            None,
            Some(5),
        );

        t1.start(o1);
        t2.start(o2);
        t3.start(o3);

        time::sleep(Duration::from_millis(700)).await;

        t1.stop();
        t2.stop();
        t3.stop();

        time::sleep(Duration::from_millis(500)).await;

        // timeouts after 100ms
        let expected_msg = "timeout occurred, retry timeouts=1 timer_id=1";
        assert!(logs_contain(expected_msg));

        // timeouts after 200ms
        let expected_msg = "timeout occurred, retry timeouts=1 timer_id=2";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout occurred, retry timeouts=1 timer_id=3";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout occurred, retry timeouts=2 timer_id=1";
        assert!(logs_contain(expected_msg));

        // timeouts after 300ms
        let expected_msg = "timeout occurred, retry timeouts=3 timer_id=1";
        assert!(logs_contain(expected_msg));

        // timeouts after 400ms
        let expected_msg = "timeout occurred, retry timeouts=2 timer_id=2";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout occurred, retry timeouts=2 timer_id=3";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout occurred, retry timeouts=4 timer_id=1";
        assert!(logs_contain(expected_msg));

        // timeouts after 500ms
        let expected_msg = "timeout occurred, retry timeouts=4 timer_id=1";
        assert!(logs_contain(expected_msg));

        // timeouts after 600ms
        let expected_msg = "timeout occurred, retry timeouts=3 timer_id=2";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout occurred, retry timeouts=3 timer_id=3";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout occurred, retry timeouts=5 timer_id=1";
        assert!(logs_contain(expected_msg));

        // timeouts after 700ms
        let expected_msg = "timeout occurred, stop retry timeouts=6 timer_id=1";
        assert!(logs_contain(expected_msg));

        // stop timer 2 and 3
        let expected_msg = "timer cancelled timer_id=2";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timer cancelled timer_id=3";
        assert!(logs_contain(expected_msg));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_timer_reset() {
        let o = Arc::new(Observer { id: 10 });

        let mut t = Timer::new(
            o.id,
            TimerType::Constant,
            Duration::from_millis(100),
            None,
            Some(5),
        );

        t.start(o.clone());

        time::sleep(Duration::from_millis(350)).await;

        let expected_msg = "timeout occurred, retry timeouts=3 timer_id=10";
        assert!(logs_contain(expected_msg));

        t.reset(o.clone());

        time::sleep(Duration::from_millis(250)).await;

        let expected_msg = "timeout occurred, retry timeouts=2 timer_id=10";
        assert!(logs_contain(expected_msg));

        t.reset(o.clone());

        time::sleep(Duration::from_millis(700)).await;

        let expected_msg = "timeout occurred, stop retry timeouts=6 timer_id=10";
        assert!(logs_contain(expected_msg));

        t.reset(o);

        time::sleep(Duration::from_millis(700)).await;

        let expected_msg = "timeout occurred, stop retry timeouts=6 timer_id=10";
        assert!(logs_contain(expected_msg));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_timer_reset_without_start() {
        let o = Arc::new(Observer { id: 10 });

        let mut t = Timer::new(
            o.id,
            TimerType::Constant,
            Duration::from_millis(100),
            None,
            Some(5),
        );

        t.reset(o);

        time::sleep(Duration::from_millis(350)).await;

        let expected_msg = "timeout occurred, retry timeouts=3 timer_id=10";
        assert!(logs_contain(expected_msg));
    }
}
