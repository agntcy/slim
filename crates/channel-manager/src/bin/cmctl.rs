// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! Channel Manager CTL - CLI client for interacting with the Channel Manager gRPC service.
//!
//! Usage:
//!   cmctl <command> [options]
//!
//! The gRPC connection can be configured either with:
//!   - `--server <address>` for simple insecure connections
//!   - `--client-config <file>` for full configuration (TLS, auth, keepalive, proxy, etc.)
//!     using the same YAML format as ClientConfig. The Channel Manager API itself is gRPC.
//!
//! Available commands:
//!   list-channels          List all channels
//!   list-participants      List participants in a channel
//!   create-channel         Create a new channel (MLS enabled by default)
//!   delete-channel         Delete a channel
//!   add-participant        Add participant to channel
//!   delete-participant     Remove participant from channel

use std::path::PathBuf;

use agntcy_slim_channel_manager::proto::channel_manager_service_client::ChannelManagerServiceClient;
use agntcy_slim_channel_manager::proto::{
    AddParticipantRequest, CommandResponse, CreateChannelRequest, DeleteChannelRequest,
    DeleteParticipantRequest, ListChannelsRequest, ListParticipantsRequest,
};

use anyhow::{Context, Result, bail};
use clap::Parser;
use slim_config::client::{ClientConfig, TransportChannel};
use slim_config::errors::ConfigError;

/// Channel Manager Control Tool
#[derive(Parser)]
#[command(name = "cmctl")]
#[command(about = "Channel Manager Control Tool - interact with the Channel Manager gRPC service")]
struct Args {
    /// Simple gRPC server address (e.g., http://localhost:10356).
    /// Mutually exclusive with --client-config.
    #[arg(long = "server", default_value = "http://localhost:10356")]
    server: String,

    /// Path to a YAML file with full gRPC client configuration (TLS, auth, keepalive, proxy, etc.).
    /// Websocket transport is not supported for the Channel Manager API.
    /// When provided, --server is ignored.
    #[arg(long = "client-config")]
    client_config: Option<PathBuf>,

    /// Command to execute
    #[command(subcommand)]
    command: Command,
}

#[derive(clap::Subcommand)]
enum Command {
    /// Create a new channel
    #[command(name = "create-channel")]
    CreateChannel {
        /// Channel name (org/namespace/channel)
        channel: String,
        /// Disable MLS for this channel (MLS is enabled by default)
        #[arg(long = "disable-mls", default_value_t = false)]
        disable_mls: bool,
    },

    /// Delete a channel
    #[command(name = "delete-channel")]
    DeleteChannel {
        /// Channel name (org/namespace/channel)
        channel: String,
    },

    /// Add a participant to a channel
    #[command(name = "add-participant")]
    AddParticipant {
        /// Channel name (org/namespace/channel)
        channel: String,
        /// Participant name (org/namespace/app)
        participant: String,
    },

    /// Remove a participant from a channel
    #[command(name = "delete-participant")]
    DeleteParticipant {
        /// Channel name (org/namespace/channel)
        channel: String,
        /// Participant name (org/namespace/app)
        participant: String,
    },

    /// List all channels
    #[command(name = "list-channels")]
    ListChannels,

    /// List participants in a channel
    #[command(name = "list-participants")]
    ListParticipants {
        /// Channel name (org/namespace/channel)
        channel: String,
    },
}

/// Load a ClientConfig from a YAML file
fn load_client_config(path: &PathBuf) -> Result<ClientConfig> {
    let data = std::fs::read_to_string(path)
        .with_context(|| format!("failed to read config file: {}", path.display()))?;
    let config: ClientConfig = serde_yaml::from_str(&data)
        .with_context(|| format!("failed to parse config file: {}", path.display()))?;
    Ok(config)
}

/// Create a ChannelManagerServiceClient from either --client-config or --server
async fn create_client(
    args: &Args,
) -> Result<
    ChannelManagerServiceClient<
        impl tonic::client::GrpcService<
            tonic::body::Body,
            Error: Into<tonic::codegen::StdError> + Send,
            ResponseBody: tonic::codegen::Body<
                Data = tonic::codegen::Bytes,
                Error: Into<tonic::codegen::StdError> + Send,
            > + Send
                              + 'static,
            Future: Send,
        > + Send
        + Clone
        + 'static,
    >,
> {
    // Initialize the crypto provider — needed by ClientConfig::to_channel() for TLS support
    slim_config::tls::provider::initialize_crypto_provider();

    let client_config = if let Some(config_path) = &args.client_config {
        load_client_config(config_path)?
    } else {
        ClientConfig::with_endpoint(&args.server)
    };

    let channel = match client_config
        .to_channel()
        .await
        .context("failed to create gRPC channel")?
    {
        TransportChannel::Grpc(channel) => channel,
        TransportChannel::Websocket(_) => bail!("{}", ConfigError::GrpcChannelUnsupportedTransport),
    };
    Ok(ChannelManagerServiceClient::new(channel))
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    let mut client = create_client(&args).await?;

    match args.command {
        Command::CreateChannel {
            channel,
            disable_mls,
        } => {
            let request = CreateChannelRequest {
                channel_name: channel.clone(),
                mls_enabled: !disable_mls,
            };
            let response = client
                .create_channel(request)
                .await
                .context("failed to create channel")?;
            check_command_response(
                response.into_inner(),
                &format!("Channel {channel} created successfully"),
            )
        }

        Command::DeleteChannel { channel } => {
            let request = DeleteChannelRequest {
                channel_name: channel.clone(),
            };
            let response = client
                .delete_channel(request)
                .await
                .context("failed to delete channel")?;
            check_command_response(
                response.into_inner(),
                &format!("Channel {channel} deleted successfully"),
            )
        }

        Command::AddParticipant {
            channel,
            participant,
        } => {
            let request = AddParticipantRequest {
                channel_name: channel.clone(),
                participant_name: participant.clone(),
            };
            let response = client
                .add_participant(request)
                .await
                .context("failed to add participant")?;
            check_command_response(
                response.into_inner(),
                &format!("Participant {participant} added to channel {channel}"),
            )
        }

        Command::DeleteParticipant {
            channel,
            participant,
        } => {
            let request = DeleteParticipantRequest {
                channel_name: channel.clone(),
                participant_name: participant.clone(),
            };
            let response = client
                .delete_participant(request)
                .await
                .context("failed to delete participant")?;
            check_command_response(
                response.into_inner(),
                &format!("Participant {participant} removed from channel {channel}"),
            )
        }

        Command::ListChannels => {
            let response = client
                .list_channels(ListChannelsRequest {})
                .await
                .context("failed to list channels")?;
            let resp = response.into_inner();
            if !resp.success {
                bail!(
                    "{}",
                    resp.error_msg
                        .unwrap_or_else(|| "unknown error".to_string())
                );
            }
            println!("Channels ({}):", resp.channel_name.len());
            for name in &resp.channel_name {
                println!("  - {name}");
            }
            Ok(())
        }

        Command::ListParticipants { channel } => {
            let request = ListParticipantsRequest {
                channel_name: channel.clone(),
            };
            let response = client
                .list_participants(request)
                .await
                .context("failed to list participants")?;
            let resp = response.into_inner();
            if !resp.success {
                bail!(
                    "{}",
                    resp.error_msg
                        .unwrap_or_else(|| "unknown error".to_string())
                );
            }
            println!(
                "Participants in channel {channel} ({}):",
                resp.participant_name.len()
            );
            for name in &resp.participant_name {
                println!("  - {name}");
            }
            Ok(())
        }
    }
}

/// Check a command response, printing success message or returning an error
fn check_command_response(response: CommandResponse, success_msg: &str) -> Result<()> {
    if response.success {
        println!("{success_msg}");
        Ok(())
    } else {
        bail!(
            "{}",
            response
                .error_msg
                .unwrap_or_else(|| "unknown error".to_string())
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::net::TcpListener;
    use tempfile::NamedTempFile;

    use agntcy_slim_channel_manager::proto::channel_manager_service_server::{
        ChannelManagerService, ChannelManagerServiceServer,
    };
    use agntcy_slim_channel_manager::proto::{
        CommandResponse, ListChannelsResponse, ListParticipantsResponse,
    };
    use tonic::{Request, Response, Status};

    /// Mock implementation of the ChannelManagerService for testing.
    struct MockChannelManager;

    #[tonic::async_trait]
    impl ChannelManagerService for MockChannelManager {
        async fn create_channel(
            &self,
            request: Request<CreateChannelRequest>,
        ) -> Result<Response<CommandResponse>, Status> {
            let req = request.into_inner();
            if req.channel_name == "org/ns/existing" {
                return Ok(Response::new(CommandResponse {
                    success: false,
                    error_msg: Some("channel org/ns/existing already exists".to_string()),
                }));
            }
            Ok(Response::new(CommandResponse {
                success: true,
                error_msg: None,
            }))
        }

        async fn delete_channel(
            &self,
            request: Request<DeleteChannelRequest>,
        ) -> Result<Response<CommandResponse>, Status> {
            let req = request.into_inner();
            if req.channel_name == "org/ns/missing" {
                return Ok(Response::new(CommandResponse {
                    success: false,
                    error_msg: Some("channel org/ns/missing not found".to_string()),
                }));
            }
            Ok(Response::new(CommandResponse {
                success: true,
                error_msg: None,
            }))
        }

        async fn add_participant(
            &self,
            request: Request<AddParticipantRequest>,
        ) -> Result<Response<CommandResponse>, Status> {
            let req = request.into_inner();
            if req.channel_name == "org/ns/missing" {
                return Ok(Response::new(CommandResponse {
                    success: false,
                    error_msg: Some("channel org/ns/missing not found".to_string()),
                }));
            }
            Ok(Response::new(CommandResponse {
                success: true,
                error_msg: None,
            }))
        }

        async fn delete_participant(
            &self,
            request: Request<DeleteParticipantRequest>,
        ) -> Result<Response<CommandResponse>, Status> {
            let req = request.into_inner();
            if req.channel_name == "org/ns/missing" {
                return Ok(Response::new(CommandResponse {
                    success: false,
                    error_msg: Some("channel org/ns/missing not found".to_string()),
                }));
            }
            Ok(Response::new(CommandResponse {
                success: true,
                error_msg: None,
            }))
        }

        async fn list_channels(
            &self,
            _request: Request<ListChannelsRequest>,
        ) -> Result<Response<ListChannelsResponse>, Status> {
            Ok(Response::new(ListChannelsResponse {
                success: true,
                error_msg: None,
                channel_name: vec!["org/ns/ch1".to_string(), "org/ns/ch2".to_string()],
            }))
        }

        async fn list_participants(
            &self,
            request: Request<ListParticipantsRequest>,
        ) -> Result<Response<ListParticipantsResponse>, Status> {
            let req = request.into_inner();
            if req.channel_name == "org/ns/missing" {
                return Ok(Response::new(ListParticipantsResponse {
                    success: false,
                    error_msg: Some("channel org/ns/missing not found".to_string()),
                    participant_name: vec![],
                }));
            }
            Ok(Response::new(ListParticipantsResponse {
                success: true,
                error_msg: None,
                participant_name: vec!["org/ns/p1".to_string(), "org/ns/p2".to_string()],
            }))
        }
    }

    fn reserve_port() -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").expect("failed to bind local test port");
        listener.local_addr().unwrap().port()
    }

    /// Start a mock gRPC server and return its port.
    async fn start_mock_server() -> u16 {
        let port = reserve_port();
        let addr = format!("127.0.0.1:{port}").parse().unwrap();
        let svc = ChannelManagerServiceServer::new(MockChannelManager);

        tokio::spawn(async move {
            tonic::transport::Server::builder()
                .add_service(svc)
                .serve(addr)
                .await
                .expect("mock server failed");
        });

        // Wait for server to be ready
        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
        loop {
            if std::net::TcpStream::connect(("127.0.0.1", port)).is_ok() {
                break;
            }
            if tokio::time::Instant::now() >= deadline {
                panic!("mock server did not start in time");
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }

        port
    }

    #[test]
    fn test_load_client_config_valid_yaml() {
        // Create a temporary YAML file with valid content
        let temp_file = NamedTempFile::new().expect("failed to create temp file");
        let config_content = r#"
endpoint: "http://localhost:10356"
"#;
        fs::write(temp_file.path(), config_content).expect("failed to write config");

        let result = load_client_config(&temp_file.path().to_path_buf());
        assert!(result.is_ok(), "loading valid client config should succeed");
    }

    #[test]
    fn test_load_client_config_missing_file() {
        let path = PathBuf::from("/nonexistent/config.yaml");
        let result = load_client_config(&path);
        assert!(result.is_err(), "loading from nonexistent file should fail");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("failed to read config file"),
            "error message should mention read failure"
        );
    }

    #[test]
    fn test_load_client_config_invalid_yaml() {
        // Create a temporary file with invalid YAML
        let temp_file = NamedTempFile::new().expect("failed to create temp file");
        let invalid_yaml = r#"
this: is: invalid: yaml: [
"#;
        fs::write(temp_file.path(), invalid_yaml).expect("failed to write config");

        let result = load_client_config(&temp_file.path().to_path_buf());
        assert!(result.is_err(), "loading invalid YAML should fail");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("failed to parse config file"),
            "error message should mention parse failure"
        );
    }

    #[test]
    fn test_check_command_response_success() {
        let response = CommandResponse {
            success: true,
            error_msg: None,
        };
        let result = check_command_response(response, "Operation successful");
        assert!(result.is_ok(), "successful response should return Ok");
    }

    #[test]
    fn test_check_command_response_failure_with_error_msg() {
        let response = CommandResponse {
            success: false,
            error_msg: Some("Channel not found".to_string()),
        };
        let result = check_command_response(response, "This should not appear");
        assert!(result.is_err(), "failed response should return Err");
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Channel not found"),
            "error message should be included in the error"
        );
    }

    #[test]
    fn test_check_command_response_failure_without_error_msg() {
        let response = CommandResponse {
            success: false,
            error_msg: None,
        };
        let result = check_command_response(response, "This should not appear");
        assert!(result.is_err(), "failed response should return Err");
        assert!(
            result.unwrap_err().to_string().contains("unknown error"),
            "should use default error message when error_msg is None"
        );
    }

    #[test]
    fn test_args_parsing_with_server() {
        let args = Args::try_parse_from([
            "cmctl",
            "--server",
            "http://example.com:5000",
            "list-channels",
        ]);
        assert!(args.is_ok(), "parsing valid args should succeed");
        let args = args.unwrap();
        assert_eq!(args.server, "http://example.com:5000");
        assert!(args.client_config.is_none());
    }

    #[test]
    fn test_args_parsing_with_client_config() {
        let args = Args::try_parse_from([
            "cmctl",
            "--client-config",
            "/path/to/config.yaml",
            "list-channels",
        ]);
        assert!(args.is_ok(), "parsing valid args should succeed");
        let args = args.unwrap();
        assert_eq!(
            args.client_config,
            Some(PathBuf::from("/path/to/config.yaml"))
        );
    }

    #[test]
    fn test_args_parsing_create_channel() {
        let args = Args::try_parse_from(["cmctl", "create-channel", "org/namespace/channel"]);
        assert!(args.is_ok(), "parsing create-channel should succeed");
        let args = args.unwrap();
        match args.command {
            Command::CreateChannel {
                channel,
                disable_mls,
            } => {
                assert_eq!(channel, "org/namespace/channel");
                assert!(!disable_mls, "MLS should be enabled by default");
            }
            _ => panic!("expected CreateChannel command"),
        }
    }

    #[test]
    fn test_args_parsing_create_channel_with_disable_mls() {
        let args = Args::try_parse_from([
            "cmctl",
            "create-channel",
            "org/namespace/channel",
            "--disable-mls",
        ]);
        assert!(
            args.is_ok(),
            "parsing create-channel with --disable-mls should succeed"
        );
        let args = args.unwrap();
        match args.command {
            Command::CreateChannel {
                channel,
                disable_mls,
            } => {
                assert_eq!(channel, "org/namespace/channel");
                assert!(disable_mls, "MLS should be disabled");
            }
            _ => panic!("expected CreateChannel command"),
        }
    }

    #[test]
    fn test_args_parsing_add_participant() {
        let args = Args::try_parse_from([
            "cmctl",
            "add-participant",
            "org/namespace/channel",
            "org/namespace/app",
        ]);
        assert!(args.is_ok(), "parsing add-participant should succeed");
        let args = args.unwrap();
        match args.command {
            Command::AddParticipant {
                channel,
                participant,
            } => {
                assert_eq!(channel, "org/namespace/channel");
                assert_eq!(participant, "org/namespace/app");
            }
            _ => panic!("expected AddParticipant command"),
        }
    }

    #[test]
    fn test_args_parsing_delete_participant() {
        let args = Args::try_parse_from([
            "cmctl",
            "delete-participant",
            "org/namespace/channel",
            "org/namespace/app",
        ]);
        assert!(args.is_ok(), "parsing delete-participant should succeed");
        let args = args.unwrap();
        match args.command {
            Command::DeleteParticipant {
                channel,
                participant,
            } => {
                assert_eq!(channel, "org/namespace/channel");
                assert_eq!(participant, "org/namespace/app");
            }
            _ => panic!("expected DeleteParticipant command"),
        }
    }

    #[test]
    fn test_args_parsing_delete_channel() {
        let args = Args::try_parse_from(["cmctl", "delete-channel", "org/namespace/channel"]);
        assert!(args.is_ok(), "parsing delete-channel should succeed");
        let args = args.unwrap();
        match args.command {
            Command::DeleteChannel { channel } => {
                assert_eq!(channel, "org/namespace/channel");
            }
            _ => panic!("expected DeleteChannel command"),
        }
    }

    #[test]
    fn test_args_parsing_list_channels() {
        let args = Args::try_parse_from(["cmctl", "list-channels"]);
        assert!(args.is_ok(), "parsing list-channels should succeed");
        let args = args.unwrap();
        match args.command {
            Command::ListChannels => {}
            _ => panic!("expected ListChannels command"),
        }
    }

    #[test]
    fn test_args_parsing_list_participants() {
        let args = Args::try_parse_from(["cmctl", "list-participants", "org/namespace/channel"]);
        assert!(args.is_ok(), "parsing list-participants should succeed");
        let args = args.unwrap();
        match args.command {
            Command::ListParticipants { channel } => {
                assert_eq!(channel, "org/namespace/channel");
            }
            _ => panic!("expected ListParticipants command"),
        }
    }

    #[test]
    fn test_args_default_server() {
        let args = Args::try_parse_from(["cmctl", "list-channels"]);
        assert!(args.is_ok());
        let args = args.unwrap();
        assert_eq!(
            args.server, "http://localhost:10356",
            "server should default to localhost:10356"
        );
    }

    // --- Mock server tests covering create_client + command dispatch ---

    #[tokio::test]
    async fn test_create_client_with_server_flag() {
        let port = start_mock_server().await;
        let args = Args::try_parse_from([
            "cmctl",
            "--server",
            &format!("http://127.0.0.1:{port}"),
            "list-channels",
        ])
        .unwrap();
        let result = create_client(&args).await;
        assert!(
            result.is_ok(),
            "create_client should succeed with valid server"
        );
    }

    #[tokio::test]
    async fn test_create_client_with_config_file() {
        let port = start_mock_server().await;
        let temp_file = NamedTempFile::new().expect("failed to create temp file");
        let config_content = format!(
            r#"
endpoint: "http://127.0.0.1:{port}"
"#
        );
        fs::write(temp_file.path(), &config_content).expect("failed to write config");

        let args = Args::try_parse_from([
            "cmctl",
            "--client-config",
            temp_file.path().to_str().unwrap(),
            "list-channels",
        ])
        .unwrap();
        let result = create_client(&args).await;
        assert!(
            result.is_ok(),
            "create_client should succeed with valid config file"
        );
    }

    #[tokio::test]
    async fn test_create_client_with_missing_config_file() {
        let args = Args::try_parse_from([
            "cmctl",
            "--client-config",
            "/nonexistent/config.yaml",
            "list-channels",
        ])
        .unwrap();
        let result = create_client(&args).await;
        assert!(
            result.is_err(),
            "create_client should fail with missing config"
        );
        let err = result.err().unwrap();
        assert!(err.to_string().contains("failed to read config file"));
    }

    #[tokio::test]
    async fn test_main_create_channel_success() {
        let port = start_mock_server().await;
        let args = Args::try_parse_from([
            "cmctl",
            "--server",
            &format!("http://127.0.0.1:{port}"),
            "create-channel",
            "org/ns/new-ch",
        ])
        .unwrap();
        let mut client = create_client(&args).await.unwrap();
        let resp = client
            .create_channel(CreateChannelRequest {
                channel_name: "org/ns/new-ch".to_string(),
                mls_enabled: true,
            })
            .await
            .unwrap()
            .into_inner();
        let result = check_command_response(resp, "Channel org/ns/new-ch created successfully");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_main_create_channel_duplicate() {
        let port = start_mock_server().await;
        let args = Args::try_parse_from([
            "cmctl",
            "--server",
            &format!("http://127.0.0.1:{port}"),
            "create-channel",
            "org/ns/existing",
        ])
        .unwrap();
        let mut client = create_client(&args).await.unwrap();
        let resp = client
            .create_channel(CreateChannelRequest {
                channel_name: "org/ns/existing".to_string(),
                mls_enabled: true,
            })
            .await
            .unwrap()
            .into_inner();
        let result = check_command_response(resp, "should not print");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already exists"));
    }

    #[tokio::test]
    async fn test_main_delete_channel_success() {
        let port = start_mock_server().await;
        let args = Args::try_parse_from([
            "cmctl",
            "--server",
            &format!("http://127.0.0.1:{port}"),
            "delete-channel",
            "org/ns/ch1",
        ])
        .unwrap();
        let mut client = create_client(&args).await.unwrap();
        let resp = client
            .delete_channel(DeleteChannelRequest {
                channel_name: "org/ns/ch1".to_string(),
            })
            .await
            .unwrap()
            .into_inner();
        let result = check_command_response(resp, "Channel org/ns/ch1 deleted successfully");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_main_delete_channel_not_found() {
        let port = start_mock_server().await;
        let args = Args::try_parse_from([
            "cmctl",
            "--server",
            &format!("http://127.0.0.1:{port}"),
            "delete-channel",
            "org/ns/missing",
        ])
        .unwrap();
        let mut client = create_client(&args).await.unwrap();
        let resp = client
            .delete_channel(DeleteChannelRequest {
                channel_name: "org/ns/missing".to_string(),
            })
            .await
            .unwrap()
            .into_inner();
        let result = check_command_response(resp, "should not print");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_main_add_participant_success() {
        let port = start_mock_server().await;
        let args = Args::try_parse_from([
            "cmctl",
            "--server",
            &format!("http://127.0.0.1:{port}"),
            "add-participant",
            "org/ns/ch1",
            "org/ns/p1",
        ])
        .unwrap();
        let mut client = create_client(&args).await.unwrap();
        let resp = client
            .add_participant(AddParticipantRequest {
                channel_name: "org/ns/ch1".to_string(),
                participant_name: "org/ns/p1".to_string(),
            })
            .await
            .unwrap()
            .into_inner();
        let result =
            check_command_response(resp, "Participant org/ns/p1 added to channel org/ns/ch1");
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_main_add_participant_channel_not_found() {
        let port = start_mock_server().await;
        let args = Args::try_parse_from([
            "cmctl",
            "--server",
            &format!("http://127.0.0.1:{port}"),
            "add-participant",
            "org/ns/missing",
            "org/ns/p1",
        ])
        .unwrap();
        let mut client = create_client(&args).await.unwrap();
        let resp = client
            .add_participant(AddParticipantRequest {
                channel_name: "org/ns/missing".to_string(),
                participant_name: "org/ns/p1".to_string(),
            })
            .await
            .unwrap()
            .into_inner();
        let result = check_command_response(resp, "should not print");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_main_delete_participant_success() {
        let port = start_mock_server().await;
        let args = Args::try_parse_from([
            "cmctl",
            "--server",
            &format!("http://127.0.0.1:{port}"),
            "delete-participant",
            "org/ns/ch1",
            "org/ns/p1",
        ])
        .unwrap();
        let mut client = create_client(&args).await.unwrap();
        let resp = client
            .delete_participant(DeleteParticipantRequest {
                channel_name: "org/ns/ch1".to_string(),
                participant_name: "org/ns/p1".to_string(),
            })
            .await
            .unwrap()
            .into_inner();
        let result = check_command_response(
            resp,
            "Participant org/ns/p1 removed from channel org/ns/ch1",
        );
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn test_main_delete_participant_channel_not_found() {
        let port = start_mock_server().await;
        let args = Args::try_parse_from([
            "cmctl",
            "--server",
            &format!("http://127.0.0.1:{port}"),
            "delete-participant",
            "org/ns/missing",
            "org/ns/p1",
        ])
        .unwrap();
        let mut client = create_client(&args).await.unwrap();
        let resp = client
            .delete_participant(DeleteParticipantRequest {
                channel_name: "org/ns/missing".to_string(),
                participant_name: "org/ns/p1".to_string(),
            })
            .await
            .unwrap()
            .into_inner();
        let result = check_command_response(resp, "should not print");
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("not found"));
    }

    #[tokio::test]
    async fn test_main_list_channels() {
        let port = start_mock_server().await;
        let args = Args::try_parse_from([
            "cmctl",
            "--server",
            &format!("http://127.0.0.1:{port}"),
            "list-channels",
        ])
        .unwrap();
        let mut client = create_client(&args).await.unwrap();
        let resp = client
            .list_channels(ListChannelsRequest {})
            .await
            .unwrap()
            .into_inner();
        assert!(resp.success);
        assert_eq!(resp.channel_name.len(), 2);
        assert!(resp.channel_name.contains(&"org/ns/ch1".to_string()));
        assert!(resp.channel_name.contains(&"org/ns/ch2".to_string()));
    }

    #[tokio::test]
    async fn test_main_list_participants_success() {
        let port = start_mock_server().await;
        let args = Args::try_parse_from([
            "cmctl",
            "--server",
            &format!("http://127.0.0.1:{port}"),
            "list-participants",
            "org/ns/ch1",
        ])
        .unwrap();
        let mut client = create_client(&args).await.unwrap();
        let resp = client
            .list_participants(ListParticipantsRequest {
                channel_name: "org/ns/ch1".to_string(),
            })
            .await
            .unwrap()
            .into_inner();
        assert!(resp.success);
        assert_eq!(resp.participant_name.len(), 2);
        assert!(resp.participant_name.contains(&"org/ns/p1".to_string()));
        assert!(resp.participant_name.contains(&"org/ns/p2".to_string()));
    }

    #[tokio::test]
    async fn test_main_list_participants_channel_not_found() {
        let port = start_mock_server().await;
        let args = Args::try_parse_from([
            "cmctl",
            "--server",
            &format!("http://127.0.0.1:{port}"),
            "list-participants",
            "org/ns/missing",
        ])
        .unwrap();
        let mut client = create_client(&args).await.unwrap();
        let resp = client
            .list_participants(ListParticipantsRequest {
                channel_name: "org/ns/missing".to_string(),
            })
            .await
            .unwrap()
            .into_inner();
        assert!(!resp.success);
        assert!(resp.error_msg.unwrap().contains("not found"));
    }
}
