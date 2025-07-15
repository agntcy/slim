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
use wiremock::matchers::{method, path, body_json};
use wiremock::{Mock, MockServer, ResponseTemplate};

use slim::config;
use slim_service::{
    authzen_integration::{AuthZenService, AuthZenServiceConfig},
};

mod args;

/// AuthZEN response for mock server
#[derive(serde::Serialize)]
struct MockAuthZenResponse {
    decision: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<serde_json::Value>,
}

/// Set up a mock AuthZEN PDP server with realistic authorization policies
async fn setup_mock_pdp() -> Result<MockServer, Box<dyn std::error::Error>> {
    let mock_server = MockServer::start().await;

    // Policy 1: Deny cross-organization routes (external organizations)
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .and(body_json(serde_json::json!({
            "subject": {"type": "agent", "id": serde_json::Value::Null, "properties": serde_json::Value::Null},
            "action": {"name": "route"},
            "resource": {"type": "agent_type", "id": serde_json::Value::Null, "properties": {"organization": "external"}}
        })))
        .respond_with(ResponseTemplate::new(200).set_body_json(MockAuthZenResponse {
            decision: false,
            context: Some(serde_json::json!({"policy": "deny_cross_org_routes"})),
        }))
        .mount(&mock_server)
        .await;

    // Policy 2: Deny large messages (over 5MB) - simplified matching 
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .and(|req: &wiremock::Request| {
            if let Ok(body) = std::str::from_utf8(&req.body) {
                body.contains(r#""name":"publish"#) && body.contains("10000000")
            } else {
                false
            }
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(MockAuthZenResponse {
            decision: false,
            context: Some(serde_json::json!({"policy": "deny_large_messages"})),
        }))
        .mount(&mock_server)
        .await;

    // Policy 3: Deny cross-org subscriptions - simplified matching
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .and(|req: &wiremock::Request| {
            if let Ok(body) = std::str::from_utf8(&req.body) {
                body.contains(r#""name":"subscribe"#) && body.contains(r#""organization":"external"#)
            } else {
                false
            }
        })
        .respond_with(ResponseTemplate::new(200).set_body_json(MockAuthZenResponse {
            decision: false,
            context: Some(serde_json::json!({"policy": "deny_cross_org_subscribe"})),
        }))
        .mount(&mock_server)
        .await;

    // Default policy: Allow all other requests (same-org operations)
    Mock::given(method("POST"))
        .and(path("/access/v1/evaluation"))
        .respond_with(ResponseTemplate::new(200).set_body_json(MockAuthZenResponse {
            decision: true,
            context: Some(serde_json::json!({"policy": "default_allow"})),
        }))
        .mount(&mock_server)
        .await;

    info!("🎭 Mock PDP server started at: {}", mock_server.uri());
    info!("📋 Configured policies:");
    info!("   ✅ Same-organization operations: ALLOW");
    info!("   ❌ Cross-organization routes/subscriptions: DENY"); 
    info!("   ❌ Large messages (>5MB): DENY");
    info!("   ✅ Normal operations: ALLOW");

    Ok(mock_server)
}

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

    // Set up mock PDP if enabled
    let _mock_server = if args.mock_pdp() {
        Some(setup_mock_pdp().await?)
    } else {
        None
    };

    // Determine PDP endpoint
    let pdp_endpoint = if args.mock_pdp() {
        if let Some(ref mock_server) = _mock_server {
            mock_server.uri()
        } else {
            args.pdp_endpoint()
        }
    } else {
        args.pdp_endpoint()
    };

    // Create service with AuthZEN integration
    let id = slim_config::component::id::ID::new_with_str("slim/0").unwrap();
    let mut svc = config.services.remove(&id).unwrap();

    // Configure AuthZEN integration
    let authzen_config = AuthZenServiceConfig {
        enabled: args.authzen_enabled(),
        pdp_endpoint: pdp_endpoint.clone(),
        timeout: Duration::from_secs(5),
        cache_ttl: Duration::from_secs(300),
        fallback_allow: args.fallback_allow(),
        max_retries: 3,
    };

    let authzen_service = AuthZenService::new(Some(authzen_config))
        .expect("Failed to create AuthZEN service");

    info!("🔐 AuthZEN Integration: {}", 
        if authzen_service.is_enabled() { "ENABLED" } else { "DISABLED" });
    
    if authzen_service.is_enabled() {
        if args.mock_pdp() {
            info!("🎭 Using Mock PDP: {}", pdp_endpoint);
        } else {
            info!("🏠 PDP Endpoint: {}", pdp_endpoint);
        }
        info!("🛡️  Fallback Policy: {}", 
            if args.fallback_allow() { "ALLOW (fail-open)" } else { "DENY (fail-closed)" });
        
        if !args.fallback_allow() && !args.mock_pdp() {
            info!("ℹ️  Note: Since no PDP is running, all operations will be DENIED (fail-closed security)");
        }
    }

    // Start the service
    svc.run().await?;

    // Demo scenarios
    if args.mock_pdp() {
        info!("🎬 Running demo with Mock PDP - you should see realistic authorization decisions");
    }
    info!("ℹ️  This demo focuses on AuthZEN authorization testing (actual SLIM operations skipped)");
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

    // Keep mock server alive until the end
    drop(_mock_server);

    Ok(())
}

/// Demonstrate agent creation with authorization context
async fn demo_agent_creation(
    svc: &mut slim_service::Service,
    _authzen_service: &AuthZenService,
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
    _svc: &mut slim_service::Service,
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
            // Note: Skipping actual route creation to avoid service connection errors
            info!("🛣️  (Route creation skipped in demo - authorization successful)");
        }
        Ok(false) => {
            info!("❌ Route authorization DENIED by policy");
        }
        Err(e) => {
            warn!("⚠️  Route authorization failed: {}", e);
            info!("   This is expected when no PDP is running and fallback_allow=false");
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
    _svc: &mut slim_service::Service,
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
            
            // Note: Skipping actual session creation and publishing to avoid service errors
            info!("📤 (Message publishing skipped in demo - authorization successful)");
        }
        Ok(false) => {
            info!("❌ Publish authorization DENIED by policy");
        }
        Err(e) => {
            warn!("⚠️  Publish authorization failed: {}", e);
            info!("   This is expected when no PDP is running and fallback_allow=false");
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
        Ok(false) => info!("✅ Correctly DENIED large message by policy"),
        Err(e) => {
            warn!("⚠️  Large message authorization failed: {}", e);
            info!("   This is expected when no PDP is running and fallback_allow=false");
        }
    }

    Ok(())
}

/// Demonstrate subscribe authorization
async fn demo_subscribe_authorization(
    _svc: &mut slim_service::Service,
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
            
            // Note: Skipping actual subscription to avoid service errors
            info!("📥 (Subscription skipped in demo - authorization successful)");
        }
        Ok(false) => {
            info!("❌ Subscribe authorization DENIED by policy");
        }
        Err(e) => {
            warn!("⚠️  Subscribe authorization failed: {}", e);
            info!("   This is expected when no PDP is running and fallback_allow=false");
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
        Ok(false) => info!("✅ Correctly DENIED cross-org subscription by policy"),
        Err(e) => {
            warn!("⚠️  Cross-org subscription failed: {}", e);
            info!("   This is expected when no PDP is running and fallback_allow=false");
        }
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