#!/usr/bin/env python3
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

"""Unit tests for SLIM Python bindings."""

import pytest

try:
    import slim_uniffi_bindings.generated.slim_bindings as slim
except ImportError:
    pytest.skip("SLIM bindings not generated. Run 'task build' first.", allow_module_level=True)

def test_version():
    """Test getting SLIM version."""
    version = slim.get_version()
    assert version is not None
    assert isinstance(version, str)
    assert len(version) > 0

def test_build_info():
    """Test getting build information."""
    build_info = slim.get_build_info()
    assert build_info is not None
    assert hasattr(build_info, 'version')
    assert hasattr(build_info, 'git_sha')
    assert hasattr(build_info, 'build_date')
    assert hasattr(build_info, 'profile')
    
    assert isinstance(build_info.version, str)
    assert isinstance(build_info.git_sha, str)
    assert isinstance(build_info.build_date, str)
    assert isinstance(build_info.profile, str)

def test_initialize_crypto_provider():
    """Test initializing crypto provider."""
    # Should not raise an exception
    slim.initialize_crypto_provider()
    # Safe to call multiple times
    slim.initialize_crypto_provider()

def test_create_app_with_secret():
    """Test creating an app with shared secret."""
    slim.initialize_crypto_provider()
    
    app_name = slim.Name(
        components=['org', 'test', 'app'],
        id=None
    )
    shared_secret = "test-secret-123-must-be-at-least-32-chars-long"
    
    app = slim.create_app_with_secret(app_name, shared_secret)
    assert app is not None
    
    # Check app ID
    app_id = app.id()
    assert isinstance(app_id, int)
    assert app_id > 0
    
    # Check app name
    name = app.name()
    assert name is not None
    assert name.components == ['org', 'test', 'app']
    assert name.id is not None

def test_create_app_with_invalid_secret():
    """Test creating an app with invalid shared secret."""
    slim.initialize_crypto_provider()
    
    app_name = slim.Name(
        components=['org', 'test', 'app'],
        id=None
    )
    
    # Too short secret should fail
    with pytest.raises(Exception):
        slim.create_app_with_secret(app_name, "short")

def test_name_parsing():
    """Test name structure."""
    name = slim.Name(
        components=['org', 'namespace', 'service'],
        id=None
    )
    
    assert len(name.components) == 3
    assert name.components[0] == 'org'
    assert name.components[1] == 'namespace'
    assert name.components[2] == 'service'
    assert name.id is None

def test_session_config_structure():
    """Test session configuration structure."""
    config = slim.SessionConfig(
        session_type=slim.SessionType.POINT_TO_POINT,
        enable_mls=False,
        max_retries=3,
        interval_ms=100,
        initiator=True,
        metadata={'test': 'value'}
    )
    
    assert config.session_type == slim.SessionType.POINT_TO_POINT
    assert config.enable_mls is False
    assert config.max_retries == 3
    assert config.interval_ms == 100
    assert config.initiator is True
    assert config.metadata['test'] == 'value'

def test_tls_config_structure():
    """Test TLS configuration structure."""
    tls_config = slim.TlsConfig(
        insecure=True,
        insecure_skip_verify=None,
        cert_file=None,
        key_file=None,
        ca_file=None,
        tls_version=None,
        include_system_ca_certs_pool=None
    )
    
    assert tls_config.insecure is True
    assert tls_config.insecure_skip_verify is None

def test_server_config_structure():
    """Test server configuration structure."""
    tls_config = slim.TlsConfig(
        insecure=True,
        insecure_skip_verify=None,
        cert_file=None,
        key_file=None,
        ca_file=None,
        tls_version=None,
        include_system_ca_certs_pool=None
    )
    
    server_config = slim.ServerConfig(
        endpoint='127.0.0.1:12345',
        tls=tls_config
    )
    
    assert server_config.endpoint == '127.0.0.1:12345'
    assert server_config.tls.insecure is True

def test_client_config_structure():
    """Test client configuration structure."""
    tls_config = slim.TlsConfig(
        insecure=True,
        insecure_skip_verify=None,
        cert_file=None,
        key_file=None,
        ca_file=None,
        tls_version=None,
        include_system_ca_certs_pool=None
    )
    
    client_config = slim.ClientConfig(
        endpoint='http://localhost:12345',
        tls=tls_config
    )
    
    assert client_config.endpoint == 'http://localhost:12345'
    assert client_config.tls.insecure is True

if __name__ == '__main__':
    pytest.main([__file__, '-v'])

