// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

mod tests {
    use std::{net::SocketAddr, sync::Arc};

    use anyhow::Context;
    use slim_datapath::api::ProtoName as Name;
    use slim_datapath::messages::utils::SlimHeaderFlags;
    use slim_testing::common::reserve_local_port;
    use tracing::info;
    use tracing_test::traced_test;

    use slim_config::tls::client::TlsClientConfig;
    use slim_config::tls::server::TlsServerConfig;
    use slim_config::{client::ClientConfig, server::ServerConfig};
    use slim_datapath::api::{DataPlaneServiceServer, ProtoMessage as Message};
    use slim_datapath::errors::DataPathError;
    use slim_datapath::message_processing::MessageProcessor;

    /// Poll for server readiness with exponential backoff (max 2 seconds)
    async fn wait_for_server_ready(addr: &str, max_attempts: u32) {
        for attempt in 0..max_attempts {
            match tokio::net::TcpStream::connect(addr).await {
                Ok(_) => return, // Server is ready
                Err(_) => {
                    if attempt < max_attempts - 1 {
                        let backoff =
                            std::time::Duration::from_millis(50 * (1 + attempt as u64).min(10));
                        tokio::time::sleep(backoff).await;
                    }
                }
            }
        }
    }

    async fn run_transport_roundtrip(
        server_conf: ServerConfig,
        client_conf: ClientConfig,
        connect_addr: Option<SocketAddr>,
        transport_label: &str,
    ) -> anyhow::Result<u64> {
        let processor = MessageProcessor::new();
        let server_token = processor.run_server(&server_conf).await.context(format!(
            "failed to start {transport_label} dataplane server"
        ))?;

        // Wait for server to be ready by polling instead of fixed sleep
        let addr = if let Some(addr) = connect_addr {
            addr.to_string()
        } else {
            let port = server_conf
                .endpoint
                .rsplit(':')
                .next()
                .and_then(|p| p.parse::<u16>().ok())
                .expect("failed to parse port from server endpoint");
            format!("127.0.0.1:{port}")
        };
        wait_for_server_ready(&addr, 40).await;

        let (_handle, conn_index) = processor
            .connect(client_conf, None, connect_addr)
            .await
            .context(format!("failed to connect {transport_label} client"))?;

        for _ in 0..3 {
            processor
                .send_msg(make_message("org", "namespace", "type"), conn_index)
                .await
                .context(format!(
                    "failed to send message over {transport_label} client transport"
                ))?;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        for _ in 0..3 {
            processor
                .send_msg(make_message("org", "namespace", "type"), 0)
                .await
                .context(format!(
                    "failed to send message over {transport_label} server transport"
                ))?;
        }
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let _ = processor.disconnect(conn_index);
        server_token.cancel();
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        processor
            .shutdown()
            .await
            .context(format!("failed to shutdown {transport_label} processor"))?;

        Ok(conn_index)
    }

    #[tokio::test]
    #[traced_test]
    async fn test_connection() {
        let port = reserve_local_port();
        // setup server from configuration
        let mut server_conf = ServerConfig::with_endpoint(&format!("127.0.0.1:{port}"));
        server_conf.tls_setting.insecure = true;

        let processor = MessageProcessor::new();
        let svc = Arc::new(processor);
        let msg_processor = svc.clone();
        let ep_server = server_conf
            .to_server_future(&[DataPlaneServiceServer::from_arc(svc)])
            .await
            .unwrap();

        // start server
        tokio::spawn(async move {
            if let Err(e) = ep_server.await {
                // panic to stop the test
                panic!("Server error: {:?}", e);
            }
        });

        wait_for_server_ready(&format!("127.0.0.1:{port}"), 40).await;

        // connect client
        let mut client_config = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{port}"));
        client_config.tls_setting.insecure = true;

        // create bidirectional stream
        info!("Client connected");
        let (_, conn_index) = msg_processor
            .connect(
                client_config,
                None,
                Some(SocketAddr::from(([127, 0, 0, 1], port))),
            )
            .await
            .expect("error creating channel");

        // send messages from the client
        for n in 0..5 {
            let msg = make_message("org", "namespace", "type");
            let res = msg_processor.send_msg(msg, conn_index);
            match res.await {
                Ok(_) => {
                    info!(%n, "sent message to the server");
                }
                Err(err) => {
                    panic!("error sending message {:?}", err);
                }
            };
        }

        // wait for messages to be received by the server
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // assert messages from the client were received by the server
        let expected_msg = "received message from connection conn_index=0";
        assert!(logs_contain(expected_msg));

        // send messages from server
        for n in 0..5 {
            let msg = make_message("org", "namespace", "type");
            // let's assume that the connection index is 0
            let res = msg_processor.send_msg(msg, 0).await;
            match res {
                Ok(_) => info!(%n, "sent message to the client"),
                Err(e) => panic!("error sending message {:?}", e),
            };
        }

        // wait for messages to be received by the client
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // assert messages from the server were received by the client
        let expected_msg = "received message from connection conn_index=".to_string()
            + conn_index.to_string().as_ref();
        assert!(logs_contain(&expected_msg));

        // test the local connections
        let (_conn_id, tx, mut rx) = msg_processor.register_local_connection(false).unwrap();

        // send messages from tx and verify that they are received by rx
        let msg = make_message("org", "namespace", "type");
        tx.send(Ok(msg)).await.unwrap();

        // wait for messages to be received by the server
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        // assert messages from the client were received by the server
        let expected_msg =
            "received message from connection conn_index=".to_string() + (2).to_string().as_ref();
        assert!(logs_contain(&expected_msg));

        // let's now send a message to the connection 2 in the connection table
        let msg = make_message("message-for-us", "namespace-for-us", "type-for-us");

        // clone to keep a copy
        msg_processor.send_msg(msg.clone(), 2).await.unwrap();

        // read from rx channel
        let received_msg = rx.recv().await.unwrap();

        assert!(
            received_msg.is_ok(),
            "error receiving message {:?}",
            received_msg.err()
        );

        // make sure what we received is what we sent
        assert_eq!(received_msg.unwrap(), msg);

        // try to send a subscription_from message
        let sub_form = make_sub_from_command("org", "ns", "type", 0);
        tx.send(Ok(sub_form)).await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;

        let expected_msg = "processing subscription";
        assert!(logs_contain(expected_msg));

        // try to send a forward_to message
        let fwd_to = make_fwd_to_command("org", "ns", "type", 0);
        tx.send(Ok(fwd_to)).await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        // The subscription forwarder handles forwarding to the forward connection.
        assert!(
            logs_contain("spawning subscription forwarder task")
                || logs_contain("forwarding subscription to forward connection")
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_disconnection() {
        let port = reserve_local_port();
        // setup server from configuration
        let mut server_conf = ServerConfig::with_endpoint(&format!("127.0.0.1:{port}"));
        server_conf.tls_setting.insecure = true;

        let processor = MessageProcessor::new();
        let svc = Arc::new(processor);
        let msg_processor = svc.clone();

        let ep_server = server_conf
            .to_server_future(&[DataPlaneServiceServer::from_arc(svc)])
            .await
            .unwrap();

        tokio::spawn(async move {
            if let Err(e) = ep_server.await {
                panic!("Server error: {:?}", e);
            }
        });

        wait_for_server_ready(&format!("127.0.0.1:{port}"), 40).await;

        // create a client config we will attach to the connection
        let mut client_config = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{port}"));
        client_config.tls_setting.insecure = true;

        // connect with client_config Some(...)
        let (_, conn_index) = msg_processor
            .connect(
                client_config.clone(),
                None,
                Some(SocketAddr::from(([127, 0, 0, 1], port))),
            )
            .await
            .expect("error creating channel");

        // ensure connection exists before disconnect
        assert!(msg_processor.connection_table().get(conn_index).is_some());

        // disconnect (should cancel stream and eventually remove connection)
        let _returned_cfg = msg_processor
            .disconnect(conn_index)
            .expect("disconnect should return client config");

        // wait for cancellation to propagate and stream task to drop connection
        tokio::time::sleep(tokio::time::Duration::from_millis(200)).await;

        // after disconnect the connection should be removed
        assert!(
            msg_processor.connection_table().get(conn_index).is_none(),
            "connection should be removed after disconnect"
        );
    }

    #[tokio::test]
    #[traced_test]
    async fn test_transport_roundtrip_grpc() -> anyhow::Result<()> {
        let port = reserve_local_port();
        let server_conf = ServerConfig::with_endpoint(&format!("127.0.0.1:{port}"))
            .with_tls_settings(TlsServerConfig::insecure());

        let client_conf = ClientConfig::with_endpoint(&format!("http://127.0.0.1:{port}"))
            .with_tls_setting(TlsClientConfig::insecure());

        let conn_index = run_transport_roundtrip(
            server_conf,
            client_conf,
            Some(SocketAddr::from(([127, 0, 0, 1], port))),
            "grpc",
        )
        .await?;

        assert!(logs_contain(
            "received message from connection conn_index=0"
        ));
        let expected = format!("received message from connection conn_index={conn_index}");
        assert!(logs_contain(expected.as_str()));
        Ok(())
    }

    #[tokio::test]
    #[traced_test]
    async fn test_websocket_connection_ws() -> anyhow::Result<()> {
        let port = reserve_local_port();
        let server_conf = ServerConfig::with_endpoint(&format!("ws://127.0.0.1:{port}"))
            .with_tls_settings(TlsServerConfig::insecure());

        let client_conf = ClientConfig::with_endpoint(&format!("ws://127.0.0.1:{port}"))
            .with_tls_setting(TlsClientConfig::insecure());
        let conn_index =
            run_transport_roundtrip(server_conf, client_conf, None, "websocket ws").await?;

        assert!(logs_contain(
            "received message from connection conn_index=0"
        ));
        let expected = format!("received message from connection conn_index={conn_index}");
        assert!(logs_contain(expected.as_str()));
        Ok(())
    }

    #[tokio::test]
    #[traced_test]
    async fn test_websocket_connection_wss() -> anyhow::Result<()> {
        let port = reserve_local_port();
        let grpc_tls_testdata = format!("{}/../config/testdata/grpc", env!("CARGO_MANIFEST_DIR"));

        let server_tls = TlsServerConfig::new().with_cert_and_key_file(
            &format!("{}/server.crt", grpc_tls_testdata),
            &format!("{}/server.key", grpc_tls_testdata),
        );

        let server_conf = ServerConfig::with_endpoint(&format!("wss://127.0.0.1:{port}"))
            .with_tls_settings(server_tls);

        let client_tls =
            TlsClientConfig::new().with_ca_file(&format!("{}/ca.crt", grpc_tls_testdata));

        let client_conf = ClientConfig::with_endpoint(&format!("wss://127.0.0.1:{port}"))
            .with_server_name("example1")
            .with_tls_setting(client_tls);
        let conn_index =
            run_transport_roundtrip(server_conf, client_conf, None, "websocket wss").await?;

        assert!(logs_contain(
            "received message from connection conn_index=0"
        ));
        let expected = format!("received message from connection conn_index={conn_index}");
        assert!(logs_contain(expected.as_str()));
        Ok(())
    }

    fn make_message(org: &str, ns: &str, name: &str) -> Message {
        let source = Name::from_strings([org, ns, name]).with_id(0);
        let name = Name::from_strings([org, ns, name]).with_id(1);
        Message::builder()
            .source(source)
            .destination(name)
            .build_subscribe()
            .unwrap()
    }

    fn make_sub_from_command(org: &str, ns: &str, name_str: &str, from_conn: u64) -> Message {
        let source = Name::from_strings([org, ns, name_str]).with_id(0);
        let name = Name::from_strings([org, ns, name_str]);
        Message::builder()
            .source(source)
            .destination(name)
            .flags(SlimHeaderFlags::default().with_recv_from(from_conn))
            .build_subscribe()
            .unwrap()
    }

    fn make_fwd_to_command(org: &str, ns: &str, name_str: &str, to_conn: u64) -> Message {
        let source = Name::from_strings([org, ns, name_str]).with_id(0);
        let name = Name::from_strings([org, ns, name_str]);
        Message::builder()
            .source(source)
            .destination(name)
            .flags(SlimHeaderFlags::default().with_forward_to(to_conn))
            .build_subscribe()
            .unwrap()
    }

    #[tokio::test]
    #[traced_test]
    async fn test_message_processor_shutdown() {
        // create a message processor
        let processor = MessageProcessor::new();
        // register a local connection to create background tasks
        let (_conn_id, tx, _rx) = processor.register_local_connection(false).unwrap();
        // send a message to exercise processing path
        let msg = make_message("org", "ns", "type");
        tx.send(Ok(msg)).await.unwrap();
        tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
        // first shutdown succeeds
        processor.shutdown().await.expect("shutdown should succeed");
        // second shutdown must fail with AlreadyCloseError
        let err = processor
            .shutdown()
            .await
            .expect_err("second shutdown must fail");
        assert!(
            matches!(err, DataPathError::AlreadyClosedError),
            "error must be AlreadyClosedError"
        );
    }
}
