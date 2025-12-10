#!/usr/bin/env python3
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""Simple SLIM Python bindings example.

This example demonstrates basic SLIM functionality:
- Creating an app with shared secret authentication
- Getting app information (ID, name, version)
- Creating a session
- Basic error handling
"""

import sys
import os

# Add parent directory to path to import common utilities
sys.path.insert(0, os.path.join(os.path.dirname(__file__), '..', 'common'))
from common import parse_name, create_session_config

# Import SLIM bindings from the packaged module
try:
    import slim_uniffi_bindings.generated.slim_bindings as slim
except ImportError:
    print(f"Error: Could not import SLIM bindings. Did you run 'task build'?")
    sys.exit(1)

def main():
    """Run the simple example."""
    print("=" * 60)
    print("SLIM Python Bindings - Simple Example")
    print("=" * 60)
    
    # Initialize crypto provider
    slim.initialize_crypto_provider()
    
    # Display version information
    version = slim.get_version()
    build_info = slim.get_build_info()
    print(f"\nSLIM Version: {version}")
    print(f"Build Info:")
    print(f"  Version: {build_info.version}")
    print(f"  Git SHA: {build_info.git_sha}")
    print(f"  Build Date: {build_info.build_date}")
    print(f"  Profile: {build_info.profile}")
    
    # Create an app with shared secret authentication
    app_name = parse_name("org/example/simple-app")
    shared_secret = "demo-secret-key-for-examples-must-be-at-least-32-chars"
    
    print(f"\n🔧 Creating SLIM app: {'/'.join(app_name.components)}")
    
    try:
        app = slim.create_app_with_secret(app_name, shared_secret)
        print(f"✅ App created successfully!")
        print(f"   App ID: {app.id()}")
        
        app_info = app.name()
        print(f"   App Name: {'/'.join(app_info.components)}")
        if app_info.id:
            print(f"   Name ID: {app_info.id}")
        
    except Exception as e:
        print(f"❌ Failed to create app: {e}")
        return 1
    
    # Create a session configuration
    print(f"\n🔧 Creating session configuration...")
    destination = parse_name("org/example/destination-app")
    
    session_config = create_session_config(
        session_type='PointToPoint',
        enable_mls=False,
        max_retries=3,
        interval_ms=100,
        initiator=True,
        metadata={'example': 'simple', 'version': '1.0'}
    )
    
    print(f"   Session Type: PointToPoint")
    print(f"   Destination: {'/'.join(destination.components)}")
    
    # Note: In a real scenario, you would need network connectivity
    # to actually create a session. This example just shows the API.
    print(f"\n Note: To actually create a session, you need:")
    print(f"   1. A running SLIM server")
    print(f"   2. Network connectivity")
    print(f"   3. Proper authentication setup")
    
    print("\n" + "=" * 60)
    print("Example completed successfully!")
    print("=" * 60)
    
    return 0

if __name__ == "__main__":
    sys.exit(main())

