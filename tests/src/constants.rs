// Expected log snippets for integration test assertions.

/// Remote subscription ACK received after link negotiation completes.
pub const MSG_SUBSCRIPTION_REMOTE_ACK: &str = "subscription: remote ack received";

/// Default subscription path when remote ACK is unavailable.
pub const MSG_SUBSCRIPTION_DEFAULT_PATH: &str =
    "subscription: remote ack not available, link negotiation may not have completed yet";

/// sdk-mock auto-reply queued for an inbound message.
pub const MSG_QUEUEING_REPLY: &str = "Queueing reply";

/// Expected reply payload from subscriber app "a".
pub const MSG_HELLO_FROM_A: &str = "hello from the a";

/// Client finished connecting, subscribing, and is waiting for sessions.
pub const MSG_CLIENT_READY: &str = "CLIENT: Entering message receive loop";

/// Client session handler started.
pub const MSG_SESSION_HANDLER_TASK_STARTED: &str = "Session handler task started";

/// Session closed notification.
pub const MSG_SESSION_CLOSED: &str = "session closed";

/// Test payload sent by client C in channel-manager tests.
pub const MSG_TEST_CLIENT_C_MESSAGE: &str = "hey there, I am c!";

/// Receiver app waiting for a session.
pub const MSG_WAITING_INCOMING_SESSION: &str = "Waiting for incoming session";

/// Sender app finished with all expected replies.
pub const MSG_ALL_PARTICIPANTS_REPLIED: &str = "All participants replied correctly";

/// Embedded controller gRPC server is listening.
pub const MSG_CONTROLPLANE_SERVER_STARTED: &str = "started controlplane server";

/// SLIM node registered with the external control plane.
pub const MSG_CONNECTED_TO_CONTROL_PLANE: &str = "connected to control plane";

/// External control plane process is ready.
pub const MSG_CONTROL_PLANE_STARTED: &str = "control plane started";

/// Subscription loss reported to the control plane.
pub const MSG_NOTIFY_CONTROL_PLANE_LOST_SUBSCRIPTION: &str =
    "notify control plane about lost subscription";

/// Inter-node header MAC verification failed.
pub const MSG_HEADER_INTEGRITY_FAILED: &str = "SLIM header integrity verification failed";
