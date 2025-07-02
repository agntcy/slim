// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//! AuthZEN Integration Example
//! 
//! This example demonstrates how to use SLIM's AuthZEN integration for 
//! fine-grained authorization of agent operations including route creation,
//! message publishing, and subscription management.

use std::time::Duration;

use clap::Parser;
use slim_datapath::messages::{Agent, AgentType};
use tokio::time;
use tracing::{info, warn, error};

use slim::config;
use slim_service::{
    FireAndForgetConfiguration,
    session::{self, SessionConfig},
    authzen_integration::{AuthZenService, AuthZenServiceConfig},
};

mod args;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse command line arguments
    let args = args::Args::parse();

    // Initialize configuration and tracing
    let config_file = args.config();
    let mut config = config::load_config(config_file).expect("failed to load configuration");
    let _guard = config.tracing.setup_tracing_subscriber();

    info!("🚀 Starting SLIM AuthZEN Integration Example");
    info!("📄 Config file: {}", config_file);

    // Create service with AuthZEN integration
    let id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let mut svc = config.services.remove(&id).unwrap();

    // Configure AuthZEN integration
    let authzen_config = AuthZenServiceConfig {
        enabled: args.authzen_enabled(),
        pdp_endpoint: args.pdp_endpoint(),
        timeout: Duration::from_secs(5),
        cache_ttl: Duration::from_secs(300),
        fallback_allow: args.fallback_allow(),
        max_retries: 3,
    };

    let authzen_service = AuthZenService::new(Some(authzen_config))
        .expect("Failed to create AuthZEN service");

    info!("🔐 AuthZEN Integration: {}", 
        if authzen_service.is_enabled() { "ENABLED" } else { "DISABLED" });

    // Start the service
    svc.run().await?;

    // Demo scenarios
    demo_agent_creation(&mut svc, &authzen_service).await?;
    demo_route_authorization(&mut svc, &authzen_service).await?;
    demo_publish_authorization(&mut svc, &authzen_service).await?;
    demo_subscribe_authorization(&mut svc, &authzen_service).await?;

    info!("✅ AuthZEN Integration Example completed successfully");

    // Graceful shutdown
    let signal = svc.signal();
    match time::timeout(Duration::from_secs(10), signal.drain()).await {
        Ok(()) => info!("🛑 Service shutdown completed"),
        Err(_) => error!("⏰ Timeout waiting for service shutdown"),
    }

    Ok(())
}

/// Demonstrate agent creation with authorization context
async fn demo_agent_creation(
    svc: &mut slim_service::Service,
    authzen_service: &AuthZenService,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("\n📋 === AGENT CREATION DEMO ===");

    // Create a publisher agent
    let publisher_agent = Agent::from_strings("cisco", "demo", "publisher", 1);
    info!("👤 Creating publisher agent: {}", publisher_agent);
    
    let publisher_rx = svc.create_agent(&publisher_agent).await?;
    info!("✅ Publisher agent created successfully");

    // Create a subscriber agent  
    let subscriber_agent = Agent::from_strings("cisco", "demo", "subscriber", 2);
    info!("👤 Creating subscriber agent: {}", subscriber_agent);
    
    let _subscriber_rx = svc.create_agent(&subscriber_agent).await?;
    info!("✅ Subscriber agent created successfully");

    // Create an admin agent (for demonstration)
    let admin_agent = Agent::from_strings("cisco", "admin", "controller", 3);
    info!("👤 Creating admin agent: {}", admin_agent);
    
    let _admin_rx = svc.create_agent(&admin_agent).await?;
    info!("✅ Admin agent created successfully");

    // Drop the receiver to avoid unused variable warning
    drop(publisher_rx);

    Ok(())
}

/// Demonstrate route authorization
async fn demo_route_authorization(
    svc: &mut slim_service::Service,
    authzen_service: &AuthZenService,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("\n🛣️  === ROUTE AUTHORIZATION DEMO ===");

    let publisher_agent = Agent::from_strings("cisco", "demo", "publisher", 1);
    let target_type = AgentType::from_strings("cisco", "demo", "subscriber");
    let connection_id = 12345;

    // Test route authorization
    info!("🔍 Testing route authorization for: {} -> {}", 
        publisher_agent, target_type);

    match authzen_service.authorize_route(
        &publisher_agent,
        &target_type,
        Some(connection_id),
    ).await {
        Ok(true) => {
            info!("✅ Route authorization GRANTED");
            // Actually create the route
            svc.set_route(&publisher_agent, &target_type, None, connection_id as u64).await?;
            info!("🛣️  Route established successfully");
        }
        Ok(false) => {
            warn!("❌ Route authorization DENIED");
        }
        Err(e) => {
            error!("🚨 Route authorization ERROR: {}", e);
        }
    }

    // Test unauthorized route (different organization)
    let unauthorized_target = AgentType::from_strings("external", "demo", "service");
    info!("🔍 Testing unauthorized route: {} -> {}", 
        publisher_agent, unauthorized_target);

    match authzen_service.authorize_route(
        &publisher_agent,
        &unauthorized_target,
        Some(connection_id),
    ).await {
        Ok(true) => warn!("⚠️  Unexpected: Unauthorized route was GRANTED"),
        Ok(false) => info!("✅ Correctly DENIED unauthorized route"),
        Err(e) => warn!("🚨 Route authorization ERROR: {}", e),
    }

    Ok(())
}

/// Demonstrate publish authorization
async fn demo_publish_authorization(
    svc: &mut slim_service::Service,
    authzen_service: &AuthZenService,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("\n📤 === PUBLISH AUTHORIZATION DEMO ===");

    let publisher_agent = Agent::from_strings("cisco", "demo", "publisher", 1);
    let target_type = AgentType::from_strings("cisco", "demo", "subscriber");
    let target_id = Some(2u64);
    let message_size = Some(1024usize);

    // Test publish authorization
    info!("🔍 Testing publish authorization: {} -> {} (size: {:?})", 
        publisher_agent, target_type, message_size);

    match authzen_service.authorize_publish(
        &publisher_agent,
        &target_type,
        target_id,
        message_size,
    ).await {
        Ok(true) => {
            info!("✅ Publish authorization GRANTED");
            
            // Create a session and publish a message
            let session = svc.create_session(
                &publisher_agent,
                SessionConfig::FireAndForget(FireAndForgetConfiguration::default()),
            ).await?;

            let message = "Hello from AuthZEN demo!".as_bytes().to_vec();
            svc.publish(&publisher_agent, session, &target_type, target_id, message).await?;
            info!("📤 Message published successfully");
        }
        Ok(false) => {
            warn!("❌ Publish authorization DENIED");
        }
        Err(e) => {
            error!("🚨 Publish authorization ERROR: {}", e);
        }
    }

    // Test large message (potentially denied by policy)
    let large_message_size = Some(10_000_000usize); // 10MB
    info!("🔍 Testing large message publish: {} -> {} (size: {:?})", 
        publisher_agent, target_type, large_message_size);

    match authzen_service.authorize_publish(
        &publisher_agent,
        &target_type,
        target_id,
        large_message_size,
    ).await {
        Ok(true) => warn!("⚠️  Large message was GRANTED (check policy limits)"),
        Ok(false) => info!("✅ Correctly DENIED large message"),
        Err(e) => warn!("🚨 Large message authorization ERROR: {}", e),
    }

    Ok(())
}

/// Demonstrate subscribe authorization
async fn demo_subscribe_authorization(
    svc: &mut slim_service::Service,
    authzen_service: &AuthZenService,
) -> Result<(), Box<dyn std::error::Error>> {
    info!("\n📥 === SUBSCRIBE AUTHORIZATION DEMO ===");

    let subscriber_agent = Agent::from_strings("cisco", "demo", "subscriber", 2);
    let source_type = AgentType::from_strings("cisco", "demo", "publisher");
    let source_id = Some(1u64);

    // Test subscribe authorization
    info!("🔍 Testing subscribe authorization: {} -> {}", 
        subscriber_agent, source_type);

    match authzen_service.authorize_subscribe(
        &subscriber_agent,
        &source_type,
        source_id,
    ).await {
        Ok(true) => {
            info!("✅ Subscribe authorization GRANTED");
            
            // Actually create subscription
            svc.subscribe(&subscriber_agent, &source_type, source_id, None).await?;
            info!("📥 Subscription created successfully");
        }
        Ok(false) => {
            warn!("❌ Subscribe authorization DENIED");
        }
        Err(e) => {
            error!("🚨 Subscribe authorization ERROR: {}", e);
        }
    }

    // Test cross-organization subscription (likely denied)
    let external_type = AgentType::from_strings("external", "public", "broadcast");
    info!("🔍 Testing cross-org subscription: {} -> {}", 
        subscriber_agent, external_type);

    match authzen_service.authorize_subscribe(
        &subscriber_agent,
        &external_type,
        None,
    ).await {
        Ok(true) => warn!("⚠️  Cross-org subscription was GRANTED"),
        Ok(false) => info!("✅ Correctly DENIED cross-org subscription"),
        Err(e) => warn!("🚨 Cross-org subscription ERROR: {}", e),
    }

    // Demonstrate cache performance
    info!("🔍 Testing authorization cache performance...");
    let start = std::time::Instant::now();
    
    for i in 0..5 {
        let result = authzen_service.authorize_subscribe(
            &subscriber_agent,
            &source_type,
            source_id,
        ).await;
        
        info!("  Cache test {}: {:?} (elapsed: {:?})", 
            i + 1, result.is_ok(), start.elapsed());
    }

    info!("📊 Cache performance test completed");

    Ok(())
} 