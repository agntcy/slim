// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use clap::Parser;
use tokio::sync::mpsc;

use slim::runtime::RuntimeConfiguration;
use slim_auth::shared_secret::SharedSecret;
use slim_config::component::{Component, id::ID};
use slim_config::grpc::client::ClientConfig as GrpcClientConfig;
use slim_config::grpc::server::ServerConfig as GrpcServerConfig;
use slim_config::tls::client::TlsClientConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_datapath::messages::{Agent, AgentType, utils::SlimHeaderFlags};
use slim_service::ServiceConfiguration;
use slim_service::streaming::StreamingConfiguration;
use slim_tracing::TracingConfiguration;

const DEFAULT_PUBSUB_PORT: u16 = 46357;
const DEFAULT_SERVICE_ID: &str = "slim/0";
const DEFAULT_FANOUT: u32 = 10;
const DEFAULT_SESSION_TIMEOUT_SECS: u64 = 1;
const DEFAULT_SESSION_BUFFER_SIZE: Option<u32> = Some(10);
const STARTUP_DELAY_MS: u64 = 2000;
const TASK_SYNC_DELAY_MS: u64 = 100;
const INVITATION_DELAY_MS: u64 = 1000;
const PARTICIPANT_SHUTDOWN_DELAY_MS: u64 = 1500;
const MLS_COMMIT_TIMEOUT_MS: u64 = 3000;

const MODERATOR_NAME: &str = "org/ns/moderator/1";
const CHANNEL_NAME: &str = "channel";

#[derive(Parser, Debug, Clone)]
#[command(version, about, long_about = None)]
pub struct Args {
    /// Number of participants to create (default: 2)
    #[arg(short, long, value_name = "PARTICIPANTS", default_value_t = 2)]
    participants: u32,

    /// Time between publications in milliseconds
    #[arg(short, long, value_name = "FREQUENCY", default_value_t = 1000)]
    frequency: u32,

    /// Maximum number of messages to send from moderator
    #[arg(short, long, value_name = "MAX_MESSAGES", default_value_t = 10)]
    max_messages: u64,

    /// Time interval between participant rotations in seconds
    #[arg(
        short = 'r',
        long,
        value_name = "ROTATION_INTERVAL",
        default_value_t = 5
    )]
    rotation_interval: u64,

    /// Maximum test duration in seconds
    #[arg(long, value_name = "TEST_TIMEOUT", default_value_t = 60)]
    test_timeout: u64,
}

#[derive(Debug)]
enum TaskMessage {
    ServerReady,
    ModeratorReady,
    ParticipantReady(u32),
    MessageSent(u32, u64),
    MessageReceived(u32, u64),
    ParticipantInvited(u32),
    ParticipantRemoved(u32),
}

#[derive(Debug, Clone)]
enum TestResult {
    Success(String),
    Failure(String),
}

#[derive(Debug)]
struct TestTracker {
    messages_sent: AtomicU64,
    messages_received: AtomicU64,
    participants_invited: AtomicU64,
    participants_removed: AtomicU64,
    test_results: Arc<tokio::sync::Mutex<Vec<TestResult>>>,
    start_time: std::time::Instant,
}

impl TestTracker {
    fn new() -> Self {
        Self {
            messages_sent: AtomicU64::new(0),
            messages_received: AtomicU64::new(0),
            participants_invited: AtomicU64::new(0),
            participants_removed: AtomicU64::new(0),
            test_results: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            start_time: std::time::Instant::now(),
        }
    }

    async fn add_result(&self, result: TestResult) {
        let mut results = self.test_results.lock().await;
        results.push(result);
    }

    async fn get_summary(&self) -> TestSummary {
        let results = self.test_results.lock().await;
        let successes = results
            .iter()
            .filter(|r| matches!(r, TestResult::Success(_)))
            .count();
        let failures = results
            .iter()
            .filter(|r| matches!(r, TestResult::Failure(_)))
            .count();

        TestSummary {
            total_tests: results.len(),
            successes,
            failures,
            messages_sent: self.messages_sent.load(Ordering::Relaxed),
            messages_received: self.messages_received.load(Ordering::Relaxed),
            participants_invited: self.participants_invited.load(Ordering::Relaxed),
            participants_removed: self.participants_removed.load(Ordering::Relaxed),
            duration: self.start_time.elapsed(),
            test_results: results.clone(),
        }
    }
}

#[derive(Debug)]
struct TestSummary {
    total_tests: usize,
    successes: usize,
    failures: usize,
    messages_sent: u64,
    messages_received: u64,
    participants_invited: u64,
    participants_removed: u64,
    duration: Duration,
    test_results: Vec<TestResult>,
}

#[derive(Debug)]
struct ParticipantHandle {
    id: u32,
    shutdown_tx: tokio::sync::oneshot::Sender<()>,
}

#[derive(Debug)]
struct ParticipantManager {
    participants: HashMap<u32, ParticipantHandle>,
    next_id: u32,
}

impl ParticipantManager {
    fn new() -> Self {
        Self {
            participants: HashMap::new(),
            next_id: 1,
        }
    }

    fn add_participant(&mut self, handle: ParticipantHandle) {
        let id = handle.id;
        self.participants.insert(id, handle);
        self.next_id = self.next_id.max(id + 1);
    }

    fn remove_participant(&mut self, id: u32) -> Option<ParticipantHandle> {
        self.participants.remove(&id)
    }

    fn get_oldest_id(&self) -> Option<u32> {
        self.participants.keys().min().copied()
    }

    fn next_participant_id(&mut self) -> u32 {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    fn is_empty(&self) -> bool {
        self.participants.is_empty()
    }
}

fn parse_string_name(name: &str) -> Agent {
    let parts: Vec<&str> = name.split('/').collect();
    if parts.len() != 4 {
        panic!("Invalid agent name format: {}", name);
    }
    Agent::from_strings(
        parts[0],
        parts[1],
        parts[2],
        parts[3].parse::<u64>().expect("Invalid agent ID"),
    )
}

fn create_service_configuration(
    client_config: GrpcClientConfig,
) -> Result<slim::config::ConfigResult, String> {
    let service_config = ServiceConfiguration::new().with_client(vec![client_config]);

    let svc_id = ID::new_with_str(DEFAULT_SERVICE_ID).unwrap();
    let service = service_config
        .build_server(svc_id.clone())
        .map_err(|e| format!("Failed to build service: {}", e))?;

    let mut services = HashMap::new();
    services.insert(svc_id, service);

    let config = slim::config::ConfigResult {
        tracing: TracingConfiguration::default(),
        runtime: RuntimeConfiguration::default(),
        services,
    };

    Ok(config)
}

fn create_streaming_session_config(
    channel_name: AgentType,
    mls_enabled: bool,
) -> slim_service::session::SessionConfig {
    slim_service::session::SessionConfig::Streaming(StreamingConfiguration::new(
        slim_service::session::SessionDirection::Bidirectional,
        Some(channel_name),
        true,
        DEFAULT_SESSION_BUFFER_SIZE,
        Some(Duration::from_secs(DEFAULT_SESSION_TIMEOUT_SECS)),
        mls_enabled,
    ))
}

async fn handle_task_message(msg: TaskMessage, test_tracker: &Arc<TestTracker>) {
    match msg {
        TaskMessage::MessageSent(participant_id, message_id) => {
            test_tracker.messages_sent.fetch_add(1, Ordering::Relaxed);
            test_tracker
                .add_result(TestResult::Success(format!(
                    "Message {} sent by participant {}",
                    message_id, participant_id
                )))
                .await;
        }
        TaskMessage::MessageReceived(participant_id, message_id) => {
            test_tracker
                .messages_received
                .fetch_add(1, Ordering::Relaxed);
            test_tracker
                .add_result(TestResult::Success(format!(
                    "Message {} received by participant {}",
                    message_id, participant_id
                )))
                .await;
        }
        TaskMessage::ParticipantInvited(id) => {
            test_tracker
                .participants_invited
                .fetch_add(1, Ordering::Relaxed);
            test_tracker
                .add_result(TestResult::Success(format!(
                    "Participant {} invited to MLS group",
                    id
                )))
                .await;
        }
        TaskMessage::ParticipantRemoved(id) => {
            test_tracker
                .participants_removed
                .fetch_add(1, Ordering::Relaxed);
            test_tracker
                .add_result(TestResult::Success(format!(
                    "Participant {} removed from MLS group",
                    id
                )))
                .await;
        }
        _ => {}
    }
}

async fn remove_oldest_participant(
    participant_manager: &mut ParticipantManager,
    moderator_removal_tx: &mpsc::Sender<u32>,
) -> Result<(), String> {
    if let Some(oldest_id) = participant_manager.get_oldest_id() {
        println!("Removing participant {}", oldest_id);

        // Request MLS removal from moderator
        moderator_removal_tx
            .send(oldest_id)
            .await
            .map_err(|e| e.to_string())?;

        // Wait for MLS removal to complete
        tokio::time::sleep(Duration::from_millis(MLS_COMMIT_TIMEOUT_MS)).await;

        // Remove the participant
        if let Some(old_handle) = participant_manager.remove_participant(oldest_id) {
            println!("Shutting down participant {}", oldest_id);
            let _ = old_handle.shutdown_tx.send(());
            tokio::time::sleep(Duration::from_millis(PARTICIPANT_SHUTDOWN_DELAY_MS)).await;
        }
    }

    Ok(())
}

async fn add_new_participant(
    participant_manager: &mut ParticipantManager,
    moderator_rotation_tx: &mpsc::Sender<u32>,
    tx: &mpsc::Sender<TaskMessage>,
    args: &Args,
) -> Result<(), String> {
    let new_id = participant_manager.next_participant_id();

    println!("Adding participant {}", new_id);

    // Create new participant
    let participant_tx = tx.clone();
    let participant_args = args.clone();
    let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        run_participant_task(new_id, participant_args, participant_tx, shutdown_rx).await
    });

    let participant_handle = ParticipantHandle {
        id: new_id,
        shutdown_tx,
    };

    participant_manager.add_participant(participant_handle);

    tokio::time::sleep(Duration::from_millis(STARTUP_DELAY_MS + 500)).await;

    // Notify moderator to invite the new participant
    moderator_rotation_tx
        .send(new_id)
        .await
        .map_err(|e| e.to_string())?;

    Ok(())
}

async fn rotate_participant(
    participant_manager: &mut ParticipantManager,
    moderator_removal_tx: &mpsc::Sender<u32>,
    moderator_rotation_tx: &mpsc::Sender<u32>,
    tx: &mpsc::Sender<TaskMessage>,
    args: &Args,
) -> Result<(), String> {
    if participant_manager.is_empty() {
        return Ok(());
    }

    println!("Starting participant rotation");

    remove_oldest_participant(participant_manager, moderator_removal_tx).await?;

    add_new_participant(participant_manager, moderator_rotation_tx, tx, args).await?;

    println!("Participant rotation completed");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    println!(
        "Starting MLS group test with {} participants",
        args.participants
    );

    let test_tracker = Arc::new(TestTracker::new());
    let test_timeout = Duration::from_secs(args.test_timeout);

    let result = tokio::time::timeout(test_timeout, async {
        let (tx, mut rx) = mpsc::channel::<TaskMessage>(32);
        let server_tx = tx.clone();
        let mut server_handle = tokio::spawn(async move {
            run_server_task(server_tx).await
        });
        match rx.recv().await {
            Some(TaskMessage::ServerReady) => {
                test_tracker.add_result(TestResult::Success("Server started successfully".to_string())).await;
            }
            _ => {
                test_tracker.add_result(TestResult::Failure("Server failed to start".to_string())).await;
                return Err("Server failed to start".to_string());
            }
        }
        let moderator_tx = tx.clone();
        let moderator_args = args.clone();
        let (moderator_rotation_tx, moderator_rotation_rx) = mpsc::channel::<u32>(32);
        let (moderator_removal_tx, moderator_removal_rx) = mpsc::channel::<u32>(32);
        let mut moderator_handle = tokio::spawn(async move {
            run_moderator_task(moderator_args, moderator_tx, moderator_rotation_rx, moderator_removal_rx).await
        });

        let mut participant_manager = ParticipantManager::new();

        for i in 1..=args.participants {
            let participant_tx = tx.clone();
            let participant_args = args.clone();
            let (shutdown_tx, shutdown_rx) = tokio::sync::oneshot::channel();
            tokio::spawn(async move {
                run_participant_task(i, participant_args, participant_tx, shutdown_rx).await
            });
            let participant_handle = ParticipantHandle {
                id: i,
                shutdown_tx,
            };
            participant_manager.add_participant(participant_handle);

            tokio::time::sleep(Duration::from_millis(TASK_SYNC_DELAY_MS)).await;
        }

        let mut ready_participants = 0;
        let mut moderator_ready = false;
        while ready_participants < args.participants || !moderator_ready {
            match rx.recv().await {
                Some(TaskMessage::ParticipantReady(id)) => {
                    if id <= args.participants {
                        ready_participants += 1;
                        test_tracker.add_result(TestResult::Success(format!("Participant {} ready", id))).await;
                    } else {
                        // Ignore ready messages from rotation participants during startup
                        test_tracker.add_result(TestResult::Success(format!("Rotation participant {} ready", id))).await;
                    }
                }
                Some(TaskMessage::ModeratorReady) => {
                    moderator_ready = true;
                    test_tracker.add_result(TestResult::Success("Moderator ready".to_string())).await;
                }
                Some(msg @ (TaskMessage::MessageSent(_, _) | TaskMessage::MessageReceived(_, _) | TaskMessage::ParticipantInvited(_) | TaskMessage::ParticipantRemoved(_))) => {
                    handle_task_message(msg, &test_tracker).await;
                }
                Some(msg) => {
                    test_tracker.add_result(TestResult::Failure(format!("Unexpected message: {:?}", msg))).await;
                }
                None => {
                    test_tracker.add_result(TestResult::Failure("Channel closed unexpectedly".to_string())).await;
                    return Err("Channel closed unexpectedly".to_string());
                }
            }
        }

        test_tracker.add_result(TestResult::Success("All participants and moderator ready".to_string())).await;

        let mut interval = tokio::time::interval(Duration::from_secs(args.rotation_interval));
        interval.tick().await;

        let test_end = tokio::time::Instant::now() + Duration::from_secs(args.test_timeout);
        loop {
            tokio::select! {
                msg = rx.recv() => {
                    match msg {
                        Some(msg @ (TaskMessage::MessageSent(_, _) | TaskMessage::MessageReceived(_, _) | TaskMessage::ParticipantInvited(_) | TaskMessage::ParticipantRemoved(_))) => {
                            handle_task_message(msg, &test_tracker).await;
                        }
                        Some(_) => {
                            // Ignore other messages
                        }
                        None => {
                            test_tracker.add_result(TestResult::Failure("Channel closed during test".to_string())).await;
                            break;
                        }
                    }
                }

                result = &mut server_handle => {
                    match result {
                        Ok(_) => test_tracker.add_result(TestResult::Success("Server completed".to_string())).await,
                        Err(e) => test_tracker.add_result(TestResult::Failure(format!("Server failed: {}", e))).await,
                    }
                    break;
                }
                result = &mut moderator_handle => {
                    match result {
                        Ok(_) => test_tracker.add_result(TestResult::Success("Moderator completed".to_string())).await,
                        Err(e) => test_tracker.add_result(TestResult::Failure(format!("Moderator failed: {}", e))).await,
                    }
                    break;
                }
                _ = interval.tick() => {
                    if tokio::time::Instant::now() >= test_end {
                        test_tracker.add_result(TestResult::Success("Test completed successfully".to_string())).await;
                        break;
                    }

                    // Process any pending messages before rotation
                    while let Ok(msg) = rx.try_recv() {
                        match msg {
                            TaskMessage::MessageSent(_, _) | TaskMessage::MessageReceived(_, _) | TaskMessage::ParticipantInvited(_) | TaskMessage::ParticipantRemoved(_) => {
                                handle_task_message(msg, &test_tracker).await;
                            }
                            _ => {} // Ignore other messages
                        }
                    }

                    if let Err(e) = rotate_participant(
                        &mut participant_manager,
                        &moderator_removal_tx,
                        &moderator_rotation_tx,
                        &tx,
                        &args,
                    ).await {
                        test_tracker.add_result(TestResult::Failure(format!("Rotation failed: {}", e))).await;
                    }
                }
            }
        }

        Ok::<(), String>(())
    }).await;

    match result {
        Ok(Ok(())) => {
            let summary = test_tracker.get_summary().await;
            print_test_summary(&summary);

            if summary.failures > 0 {
                std::process::exit(1);
            } else {
                std::process::exit(0);
            }
        }
        Ok(Err(e)) => {
            println!("Test failed: {}", e);
            let summary = test_tracker.get_summary().await;
            print_test_summary(&summary);
            std::process::exit(1);
        }
        Err(_) => {
            println!("Test timed out after {} seconds", args.test_timeout);
            let summary = test_tracker.get_summary().await;
            print_test_summary(&summary);

            if summary.failures > 0 {
                std::process::exit(1);
            } else {
                std::process::exit(0);
            }
        }
    }
}

fn print_test_summary(summary: &TestSummary) {
    println!("\n=== Test Summary ===");
    println!("Duration: {:?}", summary.duration);
    println!("Total Tests: {}", summary.total_tests);
    println!("Successes: {}", summary.successes);
    println!("Failures: {}", summary.failures);
    println!("Messages Sent: {}", summary.messages_sent);
    println!("Messages Received: {}", summary.messages_received);
    println!("Participants Invited: {}", summary.participants_invited);
    println!("Participants Removed: {}", summary.participants_removed);

    if summary.failures > 0 {
        println!("\n=== FAILURES ===");
        for result in &summary.test_results {
            if let TestResult::Failure(msg) = result {
                println!("{}", msg);
            }
        }
    }

    if summary.successes > 0 {
        println!("\n=== SUCCESSES ===");
        for result in &summary.test_results {
            if let TestResult::Success(msg) = result {
                println!("{}", msg);
            }
        }
    }

    if summary.failures == 0 {
        println!("\nAll tests passed!");
    } else {
        println!("\n{} test(s) failed!", summary.failures);
    }
}

async fn run_server_task(tx: mpsc::Sender<TaskMessage>) -> Result<(), String> {
    println!("Server task starting...");

    let pubsub_server_config =
        GrpcServerConfig::with_endpoint(&format!("0.0.0.0:{}", DEFAULT_PUBSUB_PORT))
            .with_tls_settings(TlsServerConfig::default().with_insecure(true));

    let service_config = ServiceConfiguration::new().with_server(vec![pubsub_server_config]);

    let svc_id = ID::new_with_str(DEFAULT_SERVICE_ID).unwrap();
    let service = service_config
        .build_server(svc_id.clone())
        .map_err(|e| format!("Failed to build server: {}", e))?;

    let mut services = HashMap::new();
    services.insert(svc_id, service);

    let mut server_config = slim::config::ConfigResult {
        tracing: TracingConfiguration::default(),
        runtime: RuntimeConfiguration::default(),
        services,
    };

    let _guard = server_config.tracing.setup_tracing_subscriber();

    for service in server_config.services.iter_mut() {
        println!("Starting service: {}", service.0);
        service
            .1
            .start()
            .await
            .map_err(|e| format!("Failed to start service {}: {}", service.0, e))?;
    }

    tx.send(TaskMessage::ServerReady)
        .await
        .map_err(|e| e.to_string())?;
    println!(
        "Server task is ready and listening on port {}",
        DEFAULT_PUBSUB_PORT
    );

    let _services: Vec<_> = server_config.services.into_iter().collect();

    tokio::select! {
        _ = tokio::signal::ctrl_c() => {
            println!("Server received shutdown signal");
        }
        _ = tokio::time::sleep(Duration::from_secs(300)) => {
            println!("Server timeout after 5 minutes");
        }
    }

    Ok(())
}

async fn run_moderator_task(
    args: Args,
    tx: mpsc::Sender<TaskMessage>,
    mut rotation_rx: mpsc::Receiver<u32>,
    mut removal_rx: mpsc::Receiver<u32>,
) -> Result<(), String> {
    println!("Moderator task starting...");

    tokio::time::sleep(Duration::from_millis(1000)).await;

    let pubsub_client_config =
        GrpcClientConfig::with_endpoint(&format!("http://localhost:{}", DEFAULT_PUBSUB_PORT))
            .with_tls_setting(TlsClientConfig::default().with_insecure(true));

    let mut config = create_service_configuration(pubsub_client_config)?;

    let svc_id = ID::new_with_str(DEFAULT_SERVICE_ID).unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    let local_name_str = MODERATOR_NAME;
    let local_name = parse_string_name(local_name_str);

    let channel_name = AgentType::from_strings(CHANNEL_NAME, CHANNEL_NAME, CHANNEL_NAME);

    let (app, mut rx) = svc
        .create_app(
            &local_name,
            SharedSecret::new(local_name_str, "group"),
            SharedSecret::new(local_name_str, "group"),
        )
        .await
        .map_err(|e| format!("Failed to create moderator app: {}", e))?;

    svc.run()
        .await
        .map_err(|e| format!("Failed to run moderator service: {}", e))?;

    let conn_id = svc
        .get_connection_id(&svc.config().clients()[0].endpoint)
        .ok_or("Failed to get connection id".to_string())?;

    println!("Moderator remote connection id = {}", conn_id);

    app.subscribe(
        local_name.agent_type(),
        local_name.agent_id_option(),
        Some(conn_id),
    )
    .await
    .map_err(|e| format!("Failed to subscribe: {}", e))?;

    // Create participants list and MLS tracker
    let mut participants = vec![];
    let mut mls_participants = std::collections::HashSet::new();

    for i in 1..=args.participants {
        let p = AgentType::from_strings("org", "ns", &format!("t{}", i));
        participants.push(p.clone());

        // Add route
        app.set_route(&p, None, conn_id)
            .await
            .map_err(|e| format!("Failed to set route for participant {}: {}", i, e))?;
    }

    // Wait for the connection to be established
    tokio::time::sleep(Duration::from_millis(100)).await;

    let session_config = create_streaming_session_config(channel_name.clone(), true);
    let session_id = Some(rand::random());
    let info = app
        .create_session(session_config, session_id)
        .await
        .map_err(|e| format!("Failed to create session: {}", e))?;

    // Moderator is ready
    tx.send(TaskMessage::ModeratorReady)
        .await
        .map_err(|e| e.to_string())?;
    println!("Moderator is ready");

    tokio::time::sleep(Duration::from_millis(3000)).await;

    // Invite all participants
    println!("All participants ready, sending invitations");
    for (i, p) in participants.iter().enumerate() {
        let participant_id = i + 1;
        println!("Inviting participant {}: {}", participant_id, p);
        app.invite_participant(p, info.clone())
            .await
            .map_err(|e| format!("Failed to invite participant {}: {}", participant_id, e))?;

        mls_participants.insert(participant_id as u32);
        println!("Added participant {} to MLS tracking", participant_id);

        let _ = tx
            .send(TaskMessage::ParticipantInvited(participant_id as u32))
            .await;

        tokio::time::sleep(Duration::from_millis(INVITATION_DELAY_MS)).await;
    }

    let msg_payload_str = "Hello from the moderator. msg id: ";
    let mut message_count = 0u64;
    let mut message_interval = if args.frequency > 0 {
        Some(tokio::time::interval(Duration::from_millis(
            args.frequency as u64,
        )))
    } else {
        None
    };

    if let Some(ref mut interval) = message_interval {
        interval.tick().await;
    }

    println!("Moderator: starting message sending");

    let mut finished_sending = false;

    loop {
        tokio::select! {
            remove_participant_id = removal_rx.recv() => {
                if let Some(id) = remove_participant_id {
                    println!("Moderator: removing participant {}", id);

                    if !mls_participants.contains(&id) {
                        println!("Moderator: participant {} not in MLS group, skipping removal", id);
                        continue;
                    }

                    println!("Moderator: removing participant {} from MLS group (tracked: {:?})", id, mls_participants);

                    let participant_agent = Agent::from_strings("org", "ns", &format!("t{}", id), 1);

                    match app.remove_participant(&participant_agent, info.clone()).await {
                        Ok(_) => {
                            println!("Moderator: MLS removal successful for participant {}", id);
                            mls_participants.remove(&id);
                            let _ = tx.send(TaskMessage::ParticipantRemoved(id)).await;
                            tokio::time::sleep(Duration::from_millis(MLS_COMMIT_TIMEOUT_MS)).await;
                            println!("Moderator: MLS commit timeout completed for participant {}", id);
                        }
                        Err(e) => {
                            println!("Moderator: Failed to remove participant {} from MLS group: {}", id, e);
                            mls_participants.remove(&id);
                            let _ = tx.send(TaskMessage::ParticipantRemoved(id)).await;
                        }
                    }

                    println!("Moderator: completed removal request for participant {} (tracked: {:?})", id, mls_participants);
                } else {
                    println!("Moderator: removal channel closed");
                    break;
                }
            }
            new_participant_id = rotation_rx.recv() => {
                if let Some(id) = new_participant_id {
                    println!("Moderator: inviting participant {}", id);

                    let new_participant = AgentType::from_strings("org", "ns", &format!("t{}", id));

                    app.set_route(&new_participant, None, conn_id)
                        .await
                        .map_err(|e| format!("Failed to set route for new participant {}: {}", id, e))?;

                    match app.invite_participant(&new_participant, info.clone()).await {
                        Ok(_) => {
                            mls_participants.insert(id);
                            println!("Moderator: successfully invited participant {} (tracked: {:?})", id, mls_participants);
                            let _ = tx.send(TaskMessage::ParticipantInvited(id)).await;
                        }
                        Err(e) => {
                            println!("Moderator: Failed to invite new participant {}: {}", id, e);
                        }
                    }
                } else {
                    println!("Moderator: rotation channel closed");
                    break;
                }
            }
            _ = async {
                if !finished_sending && message_count < args.max_messages {
                    if let Some(ref mut interval) = message_interval {
                        interval.tick().await;
                    }

                    println!("Moderator: sending message {}", message_count);

                    let payload = format!("{}{}", msg_payload_str, message_count);
                    let p = payload.as_bytes().to_vec();

                    let flags = SlimHeaderFlags::new(DEFAULT_FANOUT, None, None, None, None);

                    match app.publish_with_flags(info.clone(), &channel_name, None, flags, p).await {
                        Ok(_) => {
                            let _ = tx.send(TaskMessage::MessageSent(0, message_count)).await;
                            message_count += 1;
                        }
                        Err(_) => {
                            eprintln!("Failed to publish message from moderator");
                        }
                    }

                    if message_count >= args.max_messages {
                        println!("Moderator finished sending {} messages", args.max_messages);
                        finished_sending = true;
                    }
                } else {
                    tokio::time::sleep(Duration::from_secs(1)).await;
                }
            }, if !finished_sending => {}
        }

        if finished_sending {
            break;
        }
    }

    loop {
        tokio::select! {
            msg = rx.recv() => {
                match msg {
                    None => {
                        println!("Moderator: message stream ended");
                        break;
                    }
                    Some(msg_info) => match msg_info {
                        Ok(msg) => {
                            let payload = match msg.message.get_payload() {
                                Some(c) => {
                                    let p = &c.blob;
                                    String::from_utf8(p.to_vec())
                                        .unwrap_or_else(|_| "invalid utf8".to_string())
                                }
                                None => "".to_string(),
                            };

                            println!("Moderator received message: {}", payload);
                            let _ = tx.send(TaskMessage::MessageReceived(0, 0)).await; // Use 0 as moderator ID
                        }
                        Err(e) => {
                            println!("Moderator received error message: {:?}", e);
                        }
                    },
                }
            }
            _ = tokio::time::sleep(Duration::from_secs(2)) => {
                println!("Moderator: timeout reached, ending");
                break;
            }
        }
    }

    Ok(())
}

async fn run_participant_task(
    id: u32,
    _args: Args,
    tx: mpsc::Sender<TaskMessage>,
    mut shutdown_rx: tokio::sync::oneshot::Receiver<()>,
) -> Result<(), String> {
    println!("Participant {} task starting...", id);

    tokio::time::sleep(Duration::from_millis(STARTUP_DELAY_MS)).await;

    let pubsub_client_config =
        GrpcClientConfig::with_endpoint(&format!("http://localhost:{}", DEFAULT_PUBSUB_PORT))
            .with_tls_setting(TlsClientConfig::default().with_insecure(true));

    let mut config = create_service_configuration(pubsub_client_config)?;

    let svc_id = ID::new_with_str(DEFAULT_SERVICE_ID).unwrap();
    let svc = config.services.get_mut(&svc_id).unwrap();

    let local_name_str = format!("org/ns/t{}/1", id);
    let local_name = parse_string_name(&local_name_str);

    let channel_name = AgentType::from_strings(CHANNEL_NAME, CHANNEL_NAME, CHANNEL_NAME);

    let (app, mut rx) = svc
        .create_app(
            &local_name,
            SharedSecret::new(&local_name_str, "group"),
            SharedSecret::new(&local_name_str, "group"),
        )
        .await
        .map_err(|e| format!("Failed to create participant {} app: {}", id, e))?;

    svc.run()
        .await
        .map_err(|e| format!("Failed to run participant {} service: {}", id, e))?;

    let conn_id = svc
        .get_connection_id(&svc.config().clients()[0].endpoint)
        .ok_or(format!(
            "Failed to get connection id for participant {}",
            id
        ))?;

    println!("Participant {} remote connection id = {}", id, conn_id);

    app.subscribe(
        local_name.agent_type(),
        local_name.agent_id_option(),
        Some(conn_id),
    )
    .await
    .map_err(|e| format!("Failed to subscribe for participant {}: {}", id, e))?;

    tokio::time::sleep(Duration::from_millis(TASK_SYNC_DELAY_MS)).await;

    tx.send(TaskMessage::ParticipantReady(id))
        .await
        .map_err(|e| e.to_string())?;
    println!("Participant {} task is ready", id);

    let moderator_name_str = MODERATOR_NAME;
    let moderator = parse_string_name(moderator_name_str);

    let mut msg_id = 0;
    let msg_payload_str = format!("Hello from participant {}. msg id: ", id);

    loop {
        tokio::select! {
            // Check for shutdown signal
            _ = &mut shutdown_rx => {
                println!("Participant {}: received shutdown signal, stopping immediately", id);
                break;
            }
            // Process incoming messages
            msg_result = rx.recv() => {
                match msg_result {
                    None => {
                        println!("Participant {}: end of stream", id);
                        break;
                    }
                    Some(msg_info) => match msg_info {
                        Ok(msg) => {
                            let publisher = msg.message.get_slim_header().get_source();
                            let payload = match msg.message.get_payload() {
                                Some(c) => {
                                    let blob = &c.blob;
                                    match String::from_utf8(blob.to_vec()) {
                                        Ok(p) => p,
                                        Err(e) => {
                                            println!("Participant {}: error parsing message: {}", id, e);
                                            continue;
                                        }
                                    }
                                }
                                None => "".to_string(),
                            };

                            let info = msg.info;
                            println!("Participant {} received message: {}", id, payload);

                            if publisher == moderator && !payload.is_empty() {
                                let _ = tx.send(TaskMessage::MessageReceived(id, msg_id as u64)).await;
                                println!("Participant {}: replying to moderator with message {}", id, msg_id);

                                let payload = format!("{}{}", msg_payload_str, msg_id);
                                let p = payload.as_bytes().to_vec();

                                let flags = SlimHeaderFlags::new(DEFAULT_FANOUT, None, None, None, None);
                                match app.publish_with_flags(info, &channel_name, None, flags, p).await {
                                    Ok(_) => {
                                        let _ = tx.send(TaskMessage::MessageSent(id, msg_id as u64)).await;
                                        msg_id += 1;
                                    }
                                    Err(_) => {
                                        println!("Participant {}: error sending reply", id);
                                        break;
                                    }
                                }
                            }
                        }
                        Err(e) => {
                            println!("Participant {} received error message: {:?}", id, e);
                        }
                    },
                }
            }
        }
    }

    Ok(())
}
