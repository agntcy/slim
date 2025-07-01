// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use parking_lot::Mutex;
use slim_config::component::id::ID;
use slim_config::grpc::server::ServerConfig;
use slim_config::tls::server::TlsServerConfig;
use slim_datapath::messages::Agent;
use slim_mls::{identity::FileBasedIdentityProvider, interceptor::MlsInterceptor, mls::Mls};
use slim_service::{FireAndForgetConfiguration, ServiceConfiguration, session::SessionConfig};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::time::sleep;

fn bench_message_exchange(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();

    let mut group = c.benchmark_group("message_exchange");
    group.sample_size(10);
    group.measurement_time(std::time::Duration::from_secs(15));

    for &message_count in &[1, 10, 100, 1000] {
        group.throughput(Throughput::Elements(message_count as u64));

        group.bench_with_input(
            BenchmarkId::new("without_mls", message_count),
            &message_count,
            |b, &msg_count| {
                b.iter(|| {
                    rt.block_on(async {
                        match message_exchange_without_mls(msg_count).await {
                            Ok(duration) => duration,
                            Err(e) => {
                                eprintln!("Benchmark without MLS failed: {}", e);
                                panic!("Benchmark failed: {}", e);
                            }
                        }
                    })
                })
            },
        );

        group.bench_with_input(
            BenchmarkId::new("with_mls", message_count),
            &message_count,
            |b, &msg_count| {
                b.iter(|| {
                    rt.block_on(async {
                        match message_exchange_with_mls(msg_count).await {
                            Ok(duration) => duration,
                            Err(e) => {
                                eprintln!("Benchmark with MLS failed: {}", e);
                                panic!("Benchmark failed: {}", e);
                            }
                        }
                    })
                })
            },
        );
    }
    group.finish();
}

async fn message_exchange_with_mls(
    message_count: usize,
) -> Result<Duration, Box<dyn std::error::Error + Send + Sync>> {
    let unique_id = rand::random::<u64>();

    let mls_dir = format!("/tmp/mls_bench_{}", unique_id);
    std::fs::create_dir_all(&mls_dir)?;

    let pub_identity = Arc::new(FileBasedIdentityProvider::new(format!(
        "{}/publisher",
        mls_dir
    ))?);
    let mut pub_mls = Mls::new(format!("publisher_{}", unique_id), pub_identity);
    pub_mls.initialize().await?;
    let group_id = pub_mls.create_group()?;

    let sub_identity = Arc::new(FileBasedIdentityProvider::new(format!(
        "{}/subscriber",
        mls_dir
    ))?);
    let mut sub_mls = Mls::new(format!("subscriber_{}", unique_id), sub_identity);
    sub_mls.initialize().await?;

    let key_package = sub_mls.generate_key_package()?;
    let welcome_msg = pub_mls.add_member(&group_id, &key_package)?;
    let sub_group_id = sub_mls.join_group(&welcome_msg)?;

    let tls_config = TlsServerConfig::new().with_insecure(true);
    let server_config = ServerConfig::with_endpoint("127.0.0.1:0").with_tls_settings(tls_config);
    let service_config = ServiceConfiguration::new().with_server(vec![server_config]);
    let service = Arc::new(
        service_config.build_server(
            ID::new_with_name(
                slim_service::ServiceBuilder::kind(),
                &format!("benchmark_{}", unique_id),
            )
            .unwrap(),
        )?,
    );

    let publisher_agent = Agent::from_strings("bench", "org", "publisher", unique_id);
    let subscriber_agent = Agent::from_strings("bench", "org", "subscriber", unique_id);

    let _pub_rx = service.create_agent(&publisher_agent).await?;
    let mut sub_rx = service.create_agent(&subscriber_agent).await?;

    sleep(Duration::from_millis(100)).await;

    service
        .subscribe(
            &subscriber_agent,
            subscriber_agent.agent_type(),
            Some(subscriber_agent.agent_id()),
            None,
        )
        .await?;

    sleep(Duration::from_millis(100)).await;

    let pub_session_info = service
        .create_session(
            &publisher_agent,
            SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
        )
        .await?;

    let sub_session_info = service
        .create_session(
            &subscriber_agent,
            SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
        )
        .await?;

    let pub_interceptor = MlsInterceptor::new(Arc::new(Mutex::new(pub_mls)), group_id);
    let sub_interceptor = MlsInterceptor::new(Arc::new(Mutex::new(sub_mls)), sub_group_id);

    service
        .add_session_interceptor(
            &publisher_agent,
            pub_session_info.id,
            Box::new(pub_interceptor),
        )
        .await?;

    service
        .add_session_interceptor(
            &subscriber_agent,
            sub_session_info.id,
            Box::new(sub_interceptor),
        )
        .await?;

    let start = Instant::now();

    let producer_handle = tokio::spawn({
        let service = Arc::clone(&service);
        let publisher_agent = publisher_agent.clone();
        let subscriber_agent = subscriber_agent.clone();
        let pub_session_info = pub_session_info.clone();
        async move {
            for i in 0..message_count {
                let message = format!("benchmark message {}", i).into_bytes();
                service
                    .publish(
                        &publisher_agent,
                        pub_session_info.clone(),
                        subscriber_agent.agent_type(),
                        Some(subscriber_agent.agent_id()),
                        message,
                    )
                    .await?;
            }
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        }
    });

    let consumer_handle = tokio::spawn(async move {
        for i in 0..message_count {
            match tokio::time::timeout(Duration::from_secs(5), sub_rx.recv()).await {
                Ok(Some(Ok(_msg))) => {
                    // Message received successfully
                }
                Ok(Some(Err(e))) => {
                    return Err::<(), Box<dyn std::error::Error + Send + Sync>>(
                        format!("Message {} receive error: {}", i, e).into(),
                    );
                }
                Ok(None) => {
                    return Err(format!("Channel closed at message {}", i).into());
                }
                Err(_) => {
                    return Err(
                        format!("Timeout waiting for message {} of {}", i, message_count).into(),
                    );
                }
            }
        }
        Ok(())
    });

    producer_handle.await??;
    consumer_handle.await??;

    let duration = start.elapsed();

    let _ = std::fs::remove_dir_all(&mls_dir);

    Ok(duration)
}

async fn message_exchange_without_mls(
    message_count: usize,
) -> Result<Duration, Box<dyn std::error::Error + Send + Sync>> {
    let unique_id = rand::random::<u64>();

    let tls_config = TlsServerConfig::new().with_insecure(true);
    let server_config = ServerConfig::with_endpoint("127.0.0.1:0").with_tls_settings(tls_config);
    let service_config = ServiceConfiguration::new().with_server(vec![server_config]);
    let service = Arc::new(
        service_config.build_server(
            ID::new_with_name(
                slim_service::ServiceBuilder::kind(),
                &format!("benchmark_{}", unique_id),
            )
            .unwrap(),
        )?,
    );

    let publisher_agent = Agent::from_strings("bench", "org", "publisher", unique_id);
    let subscriber_agent = Agent::from_strings("bench", "org", "subscriber", unique_id);

    let _pub_rx = service.create_agent(&publisher_agent).await?;
    let mut sub_rx = service.create_agent(&subscriber_agent).await?;

    sleep(Duration::from_millis(100)).await;

    service
        .subscribe(
            &subscriber_agent,
            subscriber_agent.agent_type(),
            Some(subscriber_agent.agent_id()),
            None,
        )
        .await?;

    sleep(Duration::from_millis(100)).await;

    let pub_session_info = service
        .create_session(
            &publisher_agent,
            SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
        )
        .await?;

    let _sub_session_info = service
        .create_session(
            &subscriber_agent,
            SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
        )
        .await?;

    let start = Instant::now();

    let producer_handle = tokio::spawn({
        let service = Arc::clone(&service);
        let publisher_agent = publisher_agent.clone();
        let subscriber_agent = subscriber_agent.clone();
        let pub_session_info = pub_session_info.clone();
        async move {
            for i in 0..message_count {
                let message = format!("benchmark message {}", i).into_bytes();
                service
                    .publish(
                        &publisher_agent,
                        pub_session_info.clone(),
                        subscriber_agent.agent_type(),
                        Some(subscriber_agent.agent_id()),
                        message,
                    )
                    .await?;
            }
            Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
        }
    });

    let consumer_handle = tokio::spawn(async move {
        for i in 0..message_count {
            match tokio::time::timeout(Duration::from_secs(5), sub_rx.recv()).await {
                Ok(Some(Ok(_msg))) => {
                    // Message received successfully
                }
                Ok(Some(Err(e))) => {
                    return Err::<(), Box<dyn std::error::Error + Send + Sync>>(
                        format!("Message {} receive error: {}", i, e).into(),
                    );
                }
                Ok(None) => {
                    return Err(format!("Channel closed at message {}", i).into());
                }
                Err(_) => {
                    return Err(
                        format!("Timeout waiting for message {} of {}", i, message_count).into(),
                    );
                }
            }
        }
        Ok(())
    });

    producer_handle.await??;
    consumer_handle.await??;

    let duration = start.elapsed();

    Ok(duration)
}

criterion_group!(benches, bench_message_exchange);
criterion_main!(benches);
