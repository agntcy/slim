# SLIM AuthZEN Integration Example

This example demonstrates SLIM's integration with the [OpenID AuthZEN standard](https://openid.github.io/authzen/) for fine-grained authorization of agent operations.

## What is AuthZEN?

AuthZEN is an OpenID Foundation standard that defines a REST API for Policy Decision Point (PDP) to Policy Enforcement Point (PEP) communication. It enables:

- **Fine-grained authorization**: Policy-based access control beyond simple JWT claims
- **Dynamic policies**: Real-time policy updates without service restarts  
- **Centralized management**: Single point of policy administration
- **Standards compliance**: Interoperable with any AuthZEN-compatible PDP

## Example Features

This example demonstrates:

### 🔐 Authorization Scenarios
- **Route Authorization**: Agent-to-agent route establishment permissions
- **Publish Authorization**: Message publishing with size limits and target restrictions
- **Subscribe Authorization**: Subscription permissions with cross-organization controls
- **Cache Performance**: Authorization decision caching with TTL

### 🛠️ Technical Features
- AuthZEN client configuration and integration
- SLIM entity to AuthZEN format conversion (Agent → Subject, AgentType → Resource)
- Error handling and fallback policies
- Performance testing with cached decisions

### 📊 Demo Scenarios
1. **Agent Creation**: Create publisher, subscriber, and admin agents
2. **Route Testing**: Authorized and unauthorized route creation attempts
3. **Message Publishing**: Normal and oversized message authorization
4. **Subscription Management**: Same-org and cross-org subscription attempts
5. **Cache Testing**: Performance impact of authorization caching

## Quick Start

### Prerequisites

1. **SLIM Service**: A running SLIM service instance
2. **AuthZEN PDP** (optional): An AuthZEN-compatible Policy Decision Point
   - For testing without a real PDP, the example will use fallback policies

### Running the Example

```bash
# From the data-plane/examples directory
cargo run --bin authzen-demo -- --help
```

### Basic Usage

```bash
# Run with default settings (fail-open for demo)
cargo run --bin authzen-demo

# Test fail-closed security behavior
cargo run --bin authzen-demo -- --fail-closed

# Run with a real AuthZEN PDP
cargo run --bin authzen-demo -- --pdp-endpoint http://your-pdp:8080

# Disable AuthZEN (JWT-only mode)
cargo run --bin authzen-demo -- --authzen-enabled false
```

### Command Line Options

```
Options:
  -c, --config <CONFIG>                SLIM configuration file [default: config/slim.yml]
      --authzen-enabled <BOOLEAN>      Enable AuthZEN authorization [default: true]
      --pdp-endpoint <ENDPOINT>        AuthZEN PDP endpoint URL [default: http://localhost:8080]
      --fallback-allow                 Allow operations when PDP is unavailable [default: true]
      --fail-closed                    Test fail-closed security (deny when PDP unavailable)
      --demo-mode                      Run comprehensive demo scenarios [default: true]
  -v, --verbose                        Enable verbose authorization logging [default: false]
  -h, --help                           Print help information
```

## Expected Output

When running the example, you'll see output like:

**Fail-Open Mode (Default):**
```
🚀 Starting SLIM AuthZEN Integration Example
📄 Config file: config/slim.yml
🔐 AuthZEN Integration: ENABLED
🏠 PDP Endpoint: http://localhost:8080
🛡️  Fallback Policy: ALLOW (fail-open)

📋 === AGENT CREATION DEMO ===
👤 Creating publisher agent: cisco.demo.publisher.1
✅ Publisher agent created successfully
👤 Creating subscriber agent: cisco.demo.subscriber.2
✅ Subscriber agent created successfully

🛣️ === ROUTE AUTHORIZATION DEMO ===
🔍 Testing route authorization for: cisco.demo.publisher.1 -> cisco.demo.subscriber
⚠️  Falling back to ALLOW due to AuthZEN unavailability
✅ Route authorization GRANTED
🛣️ Route established successfully

📤 === PUBLISH AUTHORIZATION DEMO ===
🔍 Testing publish authorization: cisco.demo.publisher.1 -> cisco.demo.subscriber (size: Some(1024))
⚠️  Falling back to ALLOW due to AuthZEN unavailability
✅ Publish authorization GRANTED
📤 Message published successfully

📥 === SUBSCRIBE AUTHORIZATION DEMO ===
🔍 Testing subscribe authorization: cisco.demo.subscriber.2 -> cisco.demo.publisher
⚠️  Falling back to ALLOW due to AuthZEN unavailability
✅ Subscribe authorization GRANTED
📥 Subscription created successfully

📊 Cache performance test completed
✅ AuthZEN Integration Example completed successfully
🛑 Service shutdown completed
```

**Fail-Closed Mode (--fail-closed):**
```
🚀 Starting SLIM AuthZEN Integration Example
🔐 AuthZEN Integration: ENABLED
🏠 PDP Endpoint: http://localhost:8080
🛡️  Fallback Policy: DENY (fail-closed)
ℹ️  Note: Since no PDP is running, all operations will be DENIED (fail-closed security)

🛣️ === ROUTE AUTHORIZATION DEMO ===
🔍 Testing route authorization for: cisco.demo.publisher.1 -> cisco.demo.subscriber
❌ Route authorization DENIED by policy
🔍 Testing unauthorized route: cisco.demo.publisher.1 -> external.demo.service
✅ Correctly DENIED unauthorized route

📤 === PUBLISH AUTHORIZATION DEMO ===
🔍 Testing publish authorization: cisco.demo.publisher.1 -> cisco.demo.subscriber (size: Some(1024))
❌ Publish authorization DENIED by policy
🔍 Testing large message publish: cisco.demo.publisher.1 -> cisco.demo.subscriber (size: Some(10000000))
✅ Correctly DENIED large message by policy

📥 === SUBSCRIBE AUTHORIZATION DEMO ===
🔍 Testing subscribe authorization: cisco.demo.subscriber.2 -> cisco.demo.publisher
❌ Subscribe authorization DENIED by policy
🔍 Testing cross-org subscription: cisco.demo.subscriber.2 -> external.public.broadcast
✅ Correctly DENIED cross-org subscription by policy

✅ AuthZEN Integration Example completed successfully
```

## Integration Guide

To integrate AuthZEN into your own SLIM application:

### 1. Configure AuthZEN Service

```rust
use slim_service::authzen_integration::{AuthZenService, AuthZenServiceConfig};

let authzen_config = AuthZenServiceConfig {
    enabled: true,
    pdp_endpoint: "http://your-pdp:8080".to_string(),
    timeout: Duration::from_secs(5),
    cache_ttl: Duration::from_secs(300),
    fallback_allow: false, // fail-closed
    max_retries: 3,
};

let authzen_service = AuthZenService::new(Some(authzen_config))?;
```

### 2. Use Authorization Methods

```rust
// Route authorization
let allowed = authzen_service.authorize_route(
    &agent,
    &target_type,
    Some(connection_id),
).await?;

// Publish authorization  
let allowed = authzen_service.authorize_publish(
    &source_agent,
    &target_type,
    Some(target_id),
    Some(message_size),
).await?;

// Subscribe authorization
let allowed = authzen_service.authorize_subscribe(
    &subscriber_agent,
    &source_type,
    Some(source_id),
).await?;
```

### 3. Handle Authorization Results

```rust
match authzen_service.authorize_route(&agent, &target, Some(conn_id)).await {
    Ok(true) => {
        // Proceed with operation
        service.set_route(&agent, &target, None, conn_id).await?;
    }
    Ok(false) => {
        // Operation denied
        warn!("Route authorization denied");
    }
    Err(e) => {
        // Handle error (PDP unavailable, network issues, etc.)
        error!("Authorization error: {}", e);
    }
}
```

## Policy Examples

Example AuthZEN policies that would work with this demo:

### Allow Same-Organization Communication
```json
{
  "subject": {"organization": "cisco"},
  "action": {"name": "route"},
  "resource": {"organization": "cisco"},
  "decision": true
}
```

### Deny Large Messages
```json
{
  "subject": {"type": "agent"},
  "action": {"name": "publish", "properties": {"message_size": {"$gt": 1048576}}},
  "resource": {"type": "agent"},
  "decision": false
}
```

### Allow Admin Full Access
```json
{
  "subject": {"namespace": "admin"},
  "action": {"name": "*"},
  "resource": {"type": "*"},
  "decision": true
}
```

## Troubleshooting

### PDP Connection Issues
- Verify PDP endpoint is reachable
- Check network connectivity and firewall rules
- Use `--fallback-allow` for testing without real PDP

### Authorization Denied
- Check policy configuration in your PDP
- Verify agent organization/namespace/type mappings
- Enable verbose logging with `--verbose`

### Performance Issues
- Adjust cache TTL settings
- Monitor PDP response times
- Consider local policy caching

## Next Steps

1. **Set up a real AuthZEN PDP** (e.g., Open Policy Agent with AuthZEN plugin)
2. **Define authorization policies** specific to your use case
3. **Integrate AuthZEN** into your production SLIM deployment
4. **Monitor authorization decisions** and performance metrics

For more information, see the [SLIM AuthZEN Integration Proposal](../../../../docs/authzen-integration-proposal.md). 