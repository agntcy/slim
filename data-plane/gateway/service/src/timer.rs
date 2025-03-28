use std::sync::Arc;

use tonic::async_trait;

use tokio::time::{self, Duration};
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait TimerObserver {
    async fn on_timeout(&self, timer_id: u32, timeouts: u32);
    async fn on_failure(&self, timer_id: u32, timeouts: u32);
    async fn on_stop(&self, timer_id: u32);
}

#[allow(dead_code)]
#[derive(Debug)]
pub struct Timer {
    timer_id: u32,
    duration: u32,
    max_retries: u32,
    cancellation_token: CancellationToken,
}

#[allow(dead_code)]
impl Timer {
    pub fn new(timer_id: u32, duration: u32, max_retries: u32) -> Self {
        Timer {
            timer_id,
            duration,
            max_retries,
            cancellation_token: CancellationToken::new(),
        }
    }

    pub fn start<T: TimerObserver + Send + Sync + 'static>(&self, observer: Arc<T>) {
        let timer_id = self.timer_id;
        let duration = self.duration;
        let max_retries = self.max_retries;
        let cancellation_token = self.cancellation_token.clone();

        tokio::spawn(async move {
            let mut retry = 0;
            let mut timeouts = 0;

            loop {
                let timer = time::sleep(Duration::from_millis(duration as u64));
                tokio::pin!(timer);

                tokio::select! {
                    _ = timer.as_mut() => {
                        timeouts += 1;
                        if retry < max_retries {
                            observer.on_timeout(timer_id, timeouts).await;
                        } else {
                            observer.on_failure(timer_id, timeouts).await;
                            break;
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

    pub fn stop(&self) {
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
                "timeout number {} for timer id {}, retry",
                timeouts, timer_id
            );
        }

        async fn on_failure(&self, timer_id: u32, timeouts: u32) {
            debug!(
                "timeout number {} for timer id {}, stop retry",
                timeouts, timer_id
            );
        }

        async fn on_stop(&self, timer_id: u32) {
            debug!("timer id {} cancelled", timer_id);
        }
    }

    #[tokio::test]
    #[traced_test]
    async fn test_timer() {
        let o = Arc::new(Observer { id: 10 });

        let t = Timer::new(o.id, 100, 3);

        t.start(o);

        time::sleep(Duration::from_millis(500)).await;

        // check logs to validate the test
        let expected_msg = "timeout number 1 for timer id 10, retry";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout number 2 for timer id 10, retry";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout number 3 for timer id 10, retry";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout number 4 for timer id 10, stop retry";
        assert!(logs_contain(expected_msg));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_timer_stop() {
        let o = Arc::new(Observer { id: 10 });

        let t = Timer::new(o.id, 100, 5);

        t.start(o);

        time::sleep(Duration::from_millis(350)).await;

        t.stop();

        time::sleep(Duration::from_millis(500)).await;

        // check logs to validate the test
        let expected_msg = "timeout number 1 for timer id 10, retry";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout number 2 for timer id 10, retry";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout number 3 for timer id 10, retry";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timer id 10 cancelled";
        assert!(logs_contain(expected_msg));
    }

    #[tokio::test]
    #[traced_test]
    async fn test_multiple_timers() {
        let o1 = Arc::new(Observer { id: 1 });
        let o2 = Arc::new(Observer { id: 2 });
        let o3 = Arc::new(Observer { id: 3 });

        let t1 = Timer::new(o1.id, 100, 5);
        let t2 = Timer::new(o2.id, 200, 5);
        let t3 = Timer::new(o3.id, 200, 5);

        t1.start(o1);
        t2.start(o2);
        t3.start(o3);

        time::sleep(Duration::from_millis(700)).await;

        t1.stop();
        t2.stop();
        t3.stop();

        time::sleep(Duration::from_millis(500)).await;

        // timeouts after 100ms
        let expected_msg = "timeout number 1 for timer id 1, retry";
        assert!(logs_contain(expected_msg));

        // timeouts after 200ms
        let expected_msg = "timeout number 1 for timer id 2, retry";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout number 1 for timer id 3, retry";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout number 2 for timer id 1, retry";
        assert!(logs_contain(expected_msg));

        // timeouts after 300ms
        let expected_msg = "timeout number 3 for timer id 1, retry";
        assert!(logs_contain(expected_msg));

        // timeouts after 400ms
        let expected_msg = "timeout number 2 for timer id 2, retry";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout number 2 for timer id 3, retry";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout number 4 for timer id 1, retry";
        assert!(logs_contain(expected_msg));

        // timeouts after 500ms
        let expected_msg = "timeout number 4 for timer id 1, retry";
        assert!(logs_contain(expected_msg));

        // timeouts after 600ms
        let expected_msg = "timeout number 3 for timer id 2, retry";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout number 3 for timer id 3, retry";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timeout number 5 for timer id 1, retry";
        assert!(logs_contain(expected_msg));

        // timeouts after 700ms
        let expected_msg = "timeout number 6 for timer id 1, stop retry";
        assert!(logs_contain(expected_msg));

        // stop timer 2 and 3
        let expected_msg = "timer id 2 cancelled";
        assert!(logs_contain(expected_msg));
        let expected_msg = "timer id 3 cancelled";
        assert!(logs_contain(expected_msg));
    }
}
