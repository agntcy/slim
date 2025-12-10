#!/usr/bin/env python3
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""Point-to-point SLIM Python bindings example.

This example demonstrates:
- Creating SLIM apps for sender (Bob) and receiver (Alice)
- Connecting to a SLIM server
- Creating point-to-point sessions
- Sending and receiving messages
- Proper cleanup

Usage:
    # Terminal 1 - Start Alice (receiver)
    python main.py --local="org/alice/app" --server="http://localhost:46357"
    
    # Terminal 2 - Start Bob (sender)
    python main.py --local="org/bob/app" --remote="org/alice/app" \
                   --message="Hello SLIM" --iterations=5 --server="http://localhost:46357"
"""

import sys
import os
import argparse
import time

# Add parent directory to path to import common utilities
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'common'))
from common import (
    parse_name, 
    create_client_config, 
    create_session_config,
    format_bytes
)

# Import SLIM bindings from the packaged module
try:
    import slim_uniffi_bindings.generated.slim_bindings as slim
except ImportError:
    print(f"Error: Could not import SLIM bindings. Did you run 'task build'?")
    sys.exit(1)

def run_receiver(local_name: str, server: str, shared_secret: str = "demo-secret"):
    """Run as receiver (Alice) - listens for incoming sessions and messages."""
    print("=" * 60)
    print("SLIM Point-to-Point Example - RECEIVER (Alice)")
    print("=" * 60)
    
    # Initialize crypto provider
    slim.initialize_crypto_provider()
    
    # Create app
    app_name = parse_name(local_name)
    print(f"\nCreating SLIM app: {local_name}")
    app = slim.create_app_with_secret(app_name, shared_secret)
    print(f"✅ App created (ID: {app.id()})")
    
    # Connect to server
    print(f"\nConnecting to server: {server}")
    client_config = create_client_config(server)
    conn_id = app.connect(client_config)
    print(f"✅ Connected (Connection ID: {conn_id})")
    
    try:
        # Listen for incoming session
        print(f"\n👂 Listening for incoming session...")
        session = app.listen_for_session(timeout_ms=60000)  # 60 second timeout
        print(f"✅ Session established!")
        
        dest = session.destination()
        src = session.source()
        print(f"   Source: {'/'.join(src.components)}")
        print(f"   Destination: {'/'.join(dest.components)}")
        print(f"   Session ID: {session.session_id()}")
        print(f"   Is Initiator: {session.is_initiator()}")
        
        # Receive messages
        print(f"\n📨 Receiving messages...")
        message_count = 0
        
        while True:
            try:
                msg = session.get_message(timeout_ms=10000)  # 10 second timeout
                message_count += 1
                
                print(f"\n📬 Message #{message_count} received:")
                print(f"   From: {'/'.join(msg.context.source_name.components)}")
                print(f"   Payload Type: {msg.context.payload_type}")
                print(f"   Payload: {format_bytes(msg.payload)}")
                print(f"   Connection: {msg.context.input_connection}")
                
                if msg.context.metadata:
                    print(f"   Metadata:")
                    for key, value in msg.context.metadata.items():
                        print(f"     {key}: {value}")
                
                # Send acknowledgment reply
                reply_data = f"ACK #{message_count}".encode('utf-8')
                session.publish_to(msg.context, reply_data, "text/plain", None)
                print(f"   ✅ Sent acknowledgment")
                
            except Exception as e:
                if "timeout" in str(e).lower():
                    print(f"\n⏰ Timeout waiting for message. Continuing to listen...")
                    continue
                else:
                    print(f"\n❌ Error receiving message: {e}")
                    break
        
    except Exception as e:
        print(f"\n❌ Error: {e}")
    finally:
        # Cleanup
        print(f"\n Disconnecting...")
        app.disconnect(conn_id)
        print(f"✅ Disconnected")
        print("\n" + "=" * 60)

def run_sender(local_name: str, remote_name: str, server: str, 
               message: str = "Hello", iterations: int = 5, 
               shared_secret: str = "demo-secret"):
    """Run as sender (Bob) - creates session and sends messages."""
    print("=" * 60)
    print("SLIM Point-to-Point Example - SENDER (Bob)")
    print("=" * 60)
    
    # Initialize crypto provider
    slim.initialize_crypto_provider()
    
    # Create app
    app_name = parse_name(local_name)
    print(f"\nCreating SLIM app: {local_name}")
    app = slim.create_app_with_secret(app_name, shared_secret)
    print(f"✅ App created (ID: {app.id()})")
    
    # Connect to server
    print(f"\nConnecting to server: {server}")
    client_config = create_client_config(server)
    conn_id = app.connect(client_config)
    print(f"✅ Connected (Connection ID: {conn_id})")
    
    try:
        # Set route to remote via the server connection
        destination = parse_name(remote_name)
        app.set_route(destination, conn_id)
        print(f"📍 Route set to {remote_name} via connection {conn_id}")
        
        # Create session
        session_config = create_session_config(
            session_type='PointToPoint',
            enable_mls=False,
            max_retries=3,
            interval_ms=100,
            initiator=True,
            metadata={'sender': 'bob', 'example': 'point-to-point'}
        )
        
        print(f"\n🔧 Creating session to: {remote_name}")
        session = app.create_session(session_config, destination)
        print(f"✅ Session created!")
        print(f"   Session ID: {session.session_id()}")
        print(f"   Is Initiator: {session.is_initiator()}")
        
        # Send messages
        print(f"\n📤 Sending {iterations} messages...")
        
        for i in range(iterations):
            payload = f"{message} #{i+1}".encode('utf-8')
            metadata = {'index': str(i+1), 'timestamp': str(time.time())}
            
            # Send message with completion
            completion = session.publish_with_completion(payload, "text/plain", metadata)
            print(f"   📨 Sent message #{i+1}: {message} #{i+1}")
            
            # Wait for delivery confirmation
            completion.wait()
            print(f"      ✅ Delivery confirmed")
            
            # Wait for acknowledgment
            try:
                reply = session.get_message(timeout_ms=5000)
                print(f"      📬 Received ACK: {format_bytes(reply.payload)}")
            except Exception as e:
                print(f"      ⚠️  No ACK received: {e}")
            
            if i < iterations - 1:
                time.sleep(0.5)  # Brief pause between messages
        
        print(f"\n✅ All messages sent successfully!")
        
        # Delete session
        print(f"\n Deleting session...")
        app.delete_session(session)
        print(f"✅ Session deleted")
        
    except Exception as e:
        print(f"\n❌ Error: {e}")
    finally:
        # Cleanup
        print(f"\n Disconnecting...")
        app.disconnect(conn_id)
        print(f"✅ Disconnected")
        print("\n" + "=" * 60)

def main():
    """Parse arguments and run sender or receiver."""
    parser = argparse.ArgumentParser(description='SLIM Point-to-Point Example')
    parser.add_argument('--local', required=True, help='Local app name (e.g., org/alice/app)')
    parser.add_argument('--remote', help='Remote app name (e.g., org/bob/app)')
    parser.add_argument('--server', default='http://localhost:46357', help='SLIM server endpoint')
    parser.add_argument('--message', default='Hello SLIM', help='Message to send')
    parser.add_argument('--iterations', type=int, default=5, help='Number of messages to send')
    parser.add_argument('--secret', default='demo-secret', help='Shared secret for authentication')
    
    args = parser.parse_args()
    
    if args.remote:
        # Sender mode
        run_sender(args.local, args.remote, args.server, args.message, args.iterations, args.secret)
    else:
        # Receiver mode
        run_receiver(args.local, args.server, args.secret)
    
    return 0

if __name__ == "__main__":
    sys.exit(main())

