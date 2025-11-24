#!/usr/bin/env python3
"""
OpenTelemetry HTTP receiver that forwards received data to a SLIM channel.

This receiver listens on port 4318 for OpenTelemetry metrics and traces,
then forwards them to a SLIM group session.
Supports both HTTP/1.1 and HTTP/2.
"""

import asyncio
import datetime
import json
import sys

from quart import Quart, request, jsonify
from hypercorn.config import Config
from hypercorn.asyncio import serve

import slim_bindings

from .common import (
    common_options,
    create_local_app,
    split_id,
)


# Global state for session sharing
session_ready = None
shared_session_container = None

# Create Quart app
app = Quart(__name__)


@app.route('/<path:path>', methods=['POST'])
async def handle_otel_post(path):
    """Handle POST requests from OpenTelemetry clients."""
    # Read the request body
    body = await request.get_data()
    
    # Print request information
    print(f"\n{'='*80}")
    print(f"Received POST request on: /{path}")
    print(f"Headers: {dict(request.headers)}")
    print(f"{'='*80}")
    
    # Try to decode and pretty-print if JSON, otherwise print raw
    try:
        if 'application/json' in request.headers.get('Content-Type', ''):
            body_json = json.loads(body.decode('utf-8'))
            print(json.dumps(body_json, indent=2))
        else:
            print(f"Body (raw): {body}")
    except Exception as e:
        print(f"Body (raw): {body}")
        print(f"Note: Could not parse as JSON: {e}")
    
    print(f"{'='*80}\n")
    sys.stdout.flush()
    
    # Forward to SLIM session if ready
    if session_ready and session_ready.is_set() and shared_session_container[0]:
        try:
            session = shared_session_container[0]
            await session.publish(body)
            print("-> Forwarded to SLIM session")
        except Exception as e:
            print(f"Error sending to SLIM: {e}")
    else:
        print("-> SLIM session not ready yet, message not forwarded")
    
    # Send response
    return jsonify({"status": "success"})


async def session_setup(
    local_app, created_session, session_ready, shared_session_container
):
    """
    Set up the SLIM session.

    Behavior:
      * If not moderator: wait for a new group session (listen_for_session()).
      * If moderator: reuse the created_session reference.
      * Once session is established, mark it as ready and return.
    """
    if created_session is None:
        print("Waiting for SLIM session...")
        session = await local_app.listen_for_session()
    else:
        session = created_session

    # Make session available to HTTP handler
    shared_session_container[0] = session
    session_ready.set()
    
    print(f"SLIM session ready: {session.src} -> {session.dst}")


@common_options
def run_server(
    local: str,
    slim: dict,
    remote: str | None = None,
    port: int = 4318,
    host: str = '0.0.0.0',
    enable_opentelemetry: bool = False,
    enable_mls: bool = False,
    shared_secret: str = "secret",
    jwt: str | None = None,
    spire_trust_bundle: str | None = None,
    audience: list[str] | None = None,
    spire_socket_path: str | None = None,
    spire_target_spiffe_id: str | None = None,
    spire_jwt_audience: list[str] | None = None,
    invites: list[str] | None = None,
):
    """
    Entry point for the OTel receiver (wrapped by Click).
    
    Usage:
      # As moderator (creates session and invites participants):
      otel-receiver --local org/ns/otel-receiver --remote org/ns/otel-channel \\
        --invites org/ns/participant1 --invites org/ns/participant2 \\
        --slim '{"endpoint": "http://localhost:46357", "tls": {"insecure": true}}' \\
        --shared-secret mysecret
      
      # As participant (waits for invitation):
      otel-receiver --local org/ns/participant1 \\
        --slim '{"endpoint": "http://localhost:46357", "tls": {"insecure": true}}' \\
        --shared-secret mysecret
    
    Args:
        local: Local identity string (org/ns/app).
        slim: Connection config dict (endpoint + tls).
        remote: Channel / topic identity string (org/ns/topic).
        port: HTTP server port (default: 4318).
        host: HTTP server host (default: 0.0.0.0).
        enable_opentelemetry: Activate OTEL tracing if backend available.
        enable_mls: Enable group MLS features.
        shared_secret: Shared secret for symmetric auth (demo only).
        jwt: Path to static JWT token (if using JWT auth).
        spire_trust_bundle: SPIRE trust bundle file path.
        audience: Audience list for JWT verification.
        spire_socket_path: Path to SPIRE agent socket for workload API access.
        spire_target_spiffe_id: Target SPIFFE ID for mTLS authentication with SPIRE.
        spire_jwt_audience: Audience list for SPIRE JWT-SVID validation.
        invites: List of participant IDs to invite (moderator only).
    """
    async def start():
        global session_ready, shared_session_container
        
        # Create & connect the local Slim instance
        local_app = await create_local_app(
            local,
            slim,
            enable_opentelemetry=enable_opentelemetry,
            shared_secret=shared_secret,
            jwt=jwt,
            spire_trust_bundle=spire_trust_bundle,
            audience=audience,
            spire_socket_path=spire_socket_path,
            spire_target_spiffe_id=spire_target_spiffe_id,
            spire_jwt_audience=spire_jwt_audience,
        )

        # Parse the remote channel/topic if provided
        chat_channel = split_id(remote) if remote else None

        # Session sharing between HTTP server and SLIM
        session_ready = asyncio.Event()
        shared_session_container = [None]

        # Session object only exists immediately if we are moderator
        created_session = None
        if chat_channel and invites:
            # We are the moderator; create the group session now
            print(f"Creating new SLIM group session (moderator)... {split_id(local)}")
            config = slim_bindings.SessionConfiguration.Group(
                max_retries=5,
                timeout=datetime.timedelta(seconds=5),
                mls_enabled=enable_mls,
            )

            created_session, handle = await local_app.create_session(
                chat_channel,
                config,
            )

            await handle

            # Invite each provided participant
            for invite in invites:
                invite_name = split_id(invite)
                await local_app.set_route(invite_name)
                handle = await created_session.invite(invite_name)
                await handle
                print(f"Invited {invite_name} to the group")

        # Set up SLIM session (wait for it if participant, or use created one if moderator)
        session_task = asyncio.create_task(
            session_setup(local_app, created_session, session_ready, shared_session_container)
        )
        
        # Configure Hypercorn for HTTP/2 support
        hypercorn_config = Config()
        hypercorn_config.bind = [f"{host}:{port}"]
        hypercorn_config.alpn_protocols = ['h2', 'http/1.1']  # Enable HTTP/2 and HTTP/1.1
        
        print(f"\nStarting OpenTelemetry receiver on {host}:{port}")
        print(f"Listening for POST requests on:")
        print(f"  - http://{host}:{port}/v1/traces")
        print(f"  - http://{host}:{port}/v1/metrics")
        print(f"Supporting HTTP/1.1 and HTTP/2")
        print(f"Press Ctrl+C to stop\n")
        
        try:
            # Run the server
            await serve(app, hypercorn_config)
        except KeyboardInterrupt:
            print("\n\nShutting down...")
        finally:
            session_task.cancel()
            try:
                await session_task
            except asyncio.CancelledError:
                pass
            print("Server stopped.")
    
    try:
        asyncio.run(start())
    except KeyboardInterrupt:
        print("Interrupted by user.")
