// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

mod tests {
    use std::{net::SocketAddr, sync::Arc};

    use slim_datapath::messages::utils::SlimHeaderFlags;
    use slim_datapath::messages::{Agent, AgentType};
    use tracing::info;
    use tracing_test::traced_test;

    use slim_config::grpc::{client::ClientConfig, server::ServerConfig};
    use slim_datapath::api::proto::pubsub::v1::{
        Message, pub_sub_service_server::PubSubServiceServer,
    };
    use slim_datapath::message_processing::MessageProcessor;

    #[tokio::test]
    #[traced_test]
    async fn test_connection() {
        // setup server from configuration
        let mut server_conf = ServerConfig::with_endpoint("127.0.0.1:50051");
        server_conf.tls_setting.insecure = true;

        let (processor, _signal) = MessageProcessor::new();
        let svc = Arc::new(processor);
        let msg_processor = svc.clone();
        let ep_server = server_conf
            .to_server_future(&[PubSubServiceServer::from_arc(svc)])
            .unwrap();

        // start server
        tokio::spawn(async move {
            if let Err(e) = ep_server.await {
                // panic to stop the test
                panic!("Server error: {:?}", e);
            }
        });

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // connect client
        let mut client_config = ClientConfig::with_endpoint("http://127.0.0.1:50051");
        client_config.tls_setting.insecure = true;
        let channel = client_config.to_channel().unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // create bidirectional stream
        info!("Client connected");
        let (_, conn_index) = msg_processor
            .connect(
                channel,
                None,
                None,
                Some(SocketAddr::from(([127, 0, 0, 1], 50051))),
            )
            .await
            .expect("error creating channel");

        // send messages from the client
        for n in 0..5 {
            let msg = make_message("org", "namespace", "type");
            let res = msg_processor.send_msg(msg, conn_index);
            match res.await {
                Ok(_) => {
                    info!("sent message {:?} to the server", n);
                }
                Err(err) => {
                    panic!("error sending message {:?}", err);
                }
            };
        }

        // wait for messages to be received by the server
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // assert messages from the client were received by the server
        let expected_msg = "received message from connection conn_index=0";
        assert!(logs_contain(expected_msg));

        // send messages from server
        for n in 0..5 {
            let msg = make_message("org", "namespace", "type");
            // let's assume that the connection index is 0
            let res = msg_processor.send_msg(msg, 0).await;
            match res {
                Ok(_) => info!("sent message {:?} to the client", n),
                Err(e) => panic!("error sending message {:?}", e),
            };
        }

        // wait for messages to be received by the client
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        // assert messages from the server were received by the client
        let expected_msg = "received message from connection conn_index=".to_string()
            + conn_index.to_string().as_ref();
        assert!(logs_contain(&expected_msg));

        // test the local connections
        let (_conn_id, tx, mut rx) = msg_processor.register_local_connection();

        // send messages from tx and verify that they are received by rx
        let msg = make_message("org", "namespace", "type");
        tx.send(Ok(msg)).await.unwrap();

        // wait for messages to be received by the server
        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

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

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;

        let expected_msg = "subscription update (add = true) for agent type";
        assert!(logs_contain(expected_msg));

        // try to send a forward_to message
        let fwd_to = make_fwd_to_command("org", "ns", "type", 0);
        tx.send(Ok(fwd_to)).await.unwrap();

        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
        let expected_msg = "forward subscription (add = true) to 0";
        assert!(logs_contain(expected_msg));
    }

    fn make_message(org: &str, ns: &str, agent_type: &str) -> Message {
        let source = Agent::from_strings(org, ns, agent_type, 0);
        let name = AgentType::from_strings(org, ns, agent_type);
        Message::new_subscribe(&source, &name, Some(1), None)
    }

    fn make_sub_from_command(org: &str, ns: &str, agent_type: &str, from_conn: u64) -> Message {
        let name = AgentType::from_strings(org, ns, agent_type);
        Message::new_subscribe(
            &Agent::default(),
            &name,
            None,
            Some(SlimHeaderFlags::default().with_recv_from(from_conn)),
        )
    }

    fn make_fwd_to_command(org: &str, ns: &str, agent_type: &str, to_conn: u64) -> Message {
        let source = Agent::from_strings(org, ns, agent_type, 0);
        let name = AgentType::from_strings(org, ns, agent_type);
        Message::new_subscribe(
            &source,
            &name,
            None,
            Some(SlimHeaderFlags::default().with_forward_to(to_conn)),
        )
    }
}
