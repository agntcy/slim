#!/usr/bin/env python3
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""Group (multicast) SLIM Python bindings example.

This example demonstrates:
- Creating a group session (multicast)
- Inviting participants to the session
- Broadcasting messages to all participants
- Receiving messages in a group context

Usage:
    # Terminal 1 - Start Alice (participant)
    python main.py --local="org/alice/app" --server="http://localhost:46357"
    
    # Terminal 2 - Start Bob (participant)
    python main.py --local="org/bob/app" --server="http://localhost:46357"
    
    # Terminal 3 - Start Moderator (creates group and invites)
    python main.py --local="org/moderator/app" --remote="org/default/chat-room" \
                   --invites="org/alice/app,org/bob/app" --server="http://localhost:46357"
"""

import sys
import os
import argparse
import time
import threading

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

def run_participant(local_name: str, server: str, shared_secret: str = "demo-secret"):
    """Run as participant - waits for invitation and participates in group."""
    print("=" * 60)
    print(f"SLIM Group Example - PARTICIPANT ({local_name.split('/')[-1].title()})")
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
        # Listen for incoming session (invitation)
        print(f"\n⏳ Waiting for group invitation...")
        session = app.listen_for_session(timeout_ms=120000)  # 120 second timeout
        print(f"✅ Joined group session!")
        
        dest = session.destination()
        src = session.source()
        session_type = session.session_type()
        print(f"   Source: {'/'.join(src.components)}")
        print(f"   Destination: {'/'.join(dest.components)}")
        print(f"   Session ID: {session.session_id()}")
        print(f"   Session Type: {session_type}")
        print(f"   Is Initiator: {session.is_initiator()}")
        
        # Receive messages in a separate thread
        message_count = 0
        stop_receiving = threading.Event()
        
        def receive_messages():
            nonlocal message_count
            while not stop_receiving.is_set():
                try:
                    msg = session.get_message(timeout_ms=5000)
                    message_count += 1
                    
                    print(f"\n📬 Message #{message_count} received:")
                    print(f"   From: {'/'.join(msg.context.source_name.components)}")
                    print(f"   Payload Type: {msg.context.payload_type}")
                    print(f"   Payload: {format_bytes(msg.payload)}")
                    
                    if msg.context.metadata:
                        print(f"   Metadata:")
                        for key, value in msg.context.metadata.items():
                            print(f"     {key}: {value}")
                    
                except Exception as e:
                    if "timeout" in str(e).lower():
                        continue
                    else:
                        print(f"\n❌ Error receiving: {e}")
                        break
        
        # Start receiving thread
        receiver_thread = threading.Thread(target=receive_messages, daemon=True)
        receiver_thread.start()
        
        # Send periodic messages to the group
        print(f"\n📤 Sending messages to group...")
        for i in range(3):
            time.sleep(2)  # Wait a bit before sending
            payload = f"Hello from {local_name.split('/')[-1]} #{i+1}".encode('utf-8')
            metadata = {'sender': local_name, 'index': str(i+1)}
            
            session.publish(payload, "text/plain", metadata)
            print(f"   📨 Sent: Hello from {local_name.split('/')[-1]} #{i+1}")
        
        # Keep listening for more messages
        print(f"\n👂 Listening for more messages (press Ctrl+C to stop)...")
        time.sleep(30)
        
        # Stop receiving
        stop_receiving.set()
        receiver_thread.join(timeout=2)
        
    except KeyboardInterrupt:
        print(f"\n Interrupted by user")
    except Exception as e:
        print(f"\n❌ Error: {e}")
    finally:
        # Cleanup
        print(f"\n Disconnecting...")
        app.disconnect(conn_id)
        print(f"✅ Disconnected")
        print("\n" + "=" * 60)

def run_moderator(local_name: str, remote_name: str, invites: list, server: str,
                  shared_secret: str = "demo-secret"):
    """Run as moderator - creates group and invites participants."""
    print("=" * 60)
    print("SLIM Group Example - MODERATOR")
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
        # Create group session
        destination = parse_name(remote_name)
        session_config = create_session_config(
            session_type='Group',
            enable_mls=False,
            max_retries=3,
            interval_ms=100,
            initiator=True,
            metadata={'moderator': 'true', 'room': remote_name}
        )
        
        print(f"\n Creating group session: {remote_name}")
        session = app.create_session(session_config, destination)
        print(f"✅ Group session created!")
        print(f"   Session ID: {session.session_id()}")
        print(f"   Session Type: {session.session_type()}")
        
        # Invite participants
        print(f"\n👥 Inviting participants...")
        for invite_name in invites:
            participant = parse_name(invite_name)
            
            # Set route for invitee
            try:
                app.set_route(participant, conn_id)
            except Exception as e:
                print(f"   ⚠️  Failed to set route for {invite_name}: {e}")
                continue
            
            print(f"   Inviting: {invite_name}")
            session.invite(participant)
            print(f"      ✅ Invited successfully")
            time.sleep(0.5)  # Brief pause between invites
        
        # Wait for participants to join
        print(f"\n⏳ Waiting for participants to join...")
        time.sleep(3)
        
        # Send welcome message
        print(f"\n📤 Broadcasting messages to group...")
        welcome = "Welcome to the group chat!".encode('utf-8')
        session.publish(welcome, "text/plain", {'type': 'welcome'})
        print(f"   📨 Sent welcome message")
        
        # Send a few more messages
        for i in range(3):
            time.sleep(2)
            payload = f"Moderator message #{i+1}".encode('utf-8')
            metadata = {'index': str(i+1), 'moderator': 'true'}
            session.publish(payload, "text/plain", metadata)
            print(f"   📨 Sent: Moderator message #{i+1}")
        
        # Receive messages from participants
        print(f"\n📨 Receiving messages from participants...")
        message_count = 0
        
        for _ in range(10):  # Try to receive up to 10 messages
            try:
                msg = session.get_message(timeout_ms=5000)
                message_count += 1
                
                print(f"\n📬 Message #{message_count} received:")
                print(f"   From: {'/'.join(msg.context.source_name.components)}")
                print(f"   Payload: {format_bytes(msg.payload)}")
                
                if msg.context.metadata:
                    print(f"   Metadata:")
                    for key, value in msg.context.metadata.items():
                        print(f"     {key}: {value}")
                
            except Exception as e:
                if "timeout" in str(e).lower():
                    print(f"   ⏰ Timeout - no more messages")
                    break
                else:
                    print(f"   ❌ Error: {e}")
                    break
        
        print(f"\n✅ Received {message_count} messages from participants")
        
        # Delete session
        print(f"\n Deleting session...")
        app.delete_session(session)
        print(f"✅ Session deleted")
        
    except Exception as e:
        print(f"\n❌ Error: {e}")
        import traceback
        traceback.print_exc()
    finally:
        # Cleanup
        print(f"\n Disconnecting...")
        app.disconnect(conn_id)
        print(f"✅ Disconnected")
        print("\n" + "=" * 60)

def main():
    """Parse arguments and run moderator or participant."""
    parser = argparse.ArgumentParser(description='SLIM Group Example')
    parser.add_argument('--local', required=True, help='Local app name (e.g., org/alice/app)')
    parser.add_argument('--remote', help='Group session name (e.g., org/default/chat-room)')
    parser.add_argument('--invites', help='Comma-separated list of participant names to invite')
    parser.add_argument('--server', default='http://localhost:46357', help='SLIM server endpoint')
    parser.add_argument('--secret', default='demo-secret', help='Shared secret for authentication')
    
    args = parser.parse_args()
    
    if args.remote and args.invites:
        # Moderator mode
        invite_list = [name.strip() for name in args.invites.split(',')]
        run_moderator(args.local, args.remote, invite_list, args.server, args.secret)
    else:
        # Participant mode
        run_participant(args.local, args.server, args.secret)
    
    return 0

if __name__ == "__main__":
    sys.exit(main())

