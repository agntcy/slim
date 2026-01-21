# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0
"""
Pydantic configuration models for SLIM examples.

This module provides a centralized configuration system using Pydantic,
supporting:
  - Environment variable loading
  - Config file loading (JSON, YAML, TOML)
  - Type validation
  - Sensible defaults
  - Documentation via field descriptions
"""

import os
from enum import Enum
from pathlib import Path
from typing import Any

from pydantic import Field, field_validator
from pydantic_settings import BaseSettings, SettingsConfigDict


class AuthMode(str, Enum):
    """Authentication mode for SLIM applications."""

    SHARED_SECRET = "shared_secret"
    JWT = "jwt"
    SPIRE = "spire"


class BaseConfig(BaseSettings):
    """
    Base configuration shared across all SLIM examples.

    Environment variables are prefixed with SLIM_ by default.
    Example: SLIM_LOCAL=org/ns/app
    """

    model_config = SettingsConfigDict(
        env_prefix="SLIM_",
        env_file=".env",
        env_file_encoding="utf-8",
        case_sensitive=False,
        extra="ignore",
    )

    # Core identity settings
    local: str = Field(
        ...,
        description="Local ID in the format organization/namespace/application",
    )

    remote: str | None = Field(
        None,
        description="Remote ID in the format organization/namespace/application-or-stream",
    )

    # Service connection
    slim: str = Field(
        default="http://127.0.0.1:46357",
        description="SLIM remote endpoint URL",
    )

    # Feature flags
    enable_opentelemetry: bool = Field(
        default=False,
        description="Enable OpenTelemetry tracing",
    )

    enable_mls: bool = Field(
        default=False,
        description="Enable MLS (Message Layer Security) for sessions",
    )

    # Shared secret authentication (default, not for production)
    shared_secret: str = Field(
        default="abcde-12345-fedcb-67890-deadc",
        description="Shared secret for authentication (development only)",
    )

    # JWT authentication
    jwt: str | None = Field(
        None,
        description="Path to static JWT token file for authentication",
    )

    spire_trust_bundle: str | None = Field(
        None,
        description="Path to SPIRE trust bundle file (for JWT + JWKS mode)",
    )

    audience: list[str] | None = Field(
        None,
        description="Audience list for JWT verification",
    )

    # SPIRE dynamic identity
    spire_socket_path: str | None = Field(
        None,
        description="SPIRE Workload API socket path",
    )

    spire_target_spiffe_id: str | None = Field(
        None,
        description="Target SPIFFE ID to request from SPIRE",
    )

    spire_jwt_audience: list[str] | None = Field(
        default=None,
        description="Audience(s) for SPIRE JWT SVID requests",
    )

    @field_validator("audience", mode="before")
    @classmethod
    def parse_audience(cls, v: Any) -> list[str] | None:
        """Parse audience from comma-separated string or list."""
        if v is None:
            return None
        if isinstance(v, str):
            return [a.strip() for a in v.split(",") if a.strip()]
        return v

    @field_validator("jwt", "spire_trust_bundle", mode="after")
    @classmethod
    def validate_file_paths(cls, v: str | None) -> str | None:
        """Validate that file paths exist if provided."""
        if v is not None and not Path(v).exists():
            raise ValueError(f"File not found: {v}")
        return v

    def get_auth_mode(self) -> AuthMode:
        """Determine which authentication mode to use based on provided config."""
        if (
            self.spire_socket_path
            or self.spire_target_spiffe_id
            or self.spire_jwt_audience
        ):
            return AuthMode.SPIRE
        elif self.jwt and self.spire_trust_bundle and self.audience:
            return AuthMode.JWT
        else:
            return AuthMode.SHARED_SECRET

    @classmethod
    def from_args(cls, **kwargs: Any) -> "BaseConfig":
        """
        Create config from keyword arguments.

        This allows programmatic creation while still validating fields.
        """
        # Filter out None values to let defaults take effect
        filtered = {k: v for k, v in kwargs.items() if v is not None}
        return cls(**filtered)

    @classmethod
    def from_file(cls, config_path: str) -> "BaseConfig":
        """
        Load configuration from a file (JSON, YAML, or TOML).

        Args:
            config_path: Path to configuration file

        Returns:
            Config instance with loaded values
        """
        path = Path(config_path)
        if not path.exists():
            raise FileNotFoundError(f"Config file not found: {config_path}")

        content = path.read_text()
        suffix = path.suffix.lower()

        if suffix == ".json":
            import json

            data = json.loads(content)
        elif suffix in (".yaml", ".yml"):
            try:
                import yaml  # type: ignore[import-untyped]

                data = yaml.safe_load(content)
            except ImportError:
                raise ImportError(
                    "PyYAML is required for YAML config files. Install with: pip install pyyaml"
                )
        elif suffix == ".toml":
            try:
                import tomli  # type: ignore[import-untyped]

                data = tomli.loads(content)
            except ImportError:
                raise ImportError(
                    "tomli is required for TOML config files. Install with: pip install tomli"
                )
        else:
            raise ValueError(f"Unsupported config file format: {suffix}")

        return cls(**data)


class GroupConfig(BaseConfig):
    """Configuration specific to group messaging examples."""

    invites: list[str] | None = Field(
        default=None,
        description="List of participant IDs to invite to the group session",
    )

    @field_validator("invites", mode="before")
    @classmethod
    def parse_invites(cls, v: Any) -> list[str] | None:
        """Parse invites from comma-separated string or list."""
        if v is None:
            return None
        if isinstance(v, str):
            return [i.strip() for i in v.split(",") if i.strip()]
        return v


class PointToPointConfig(BaseConfig):
    """Configuration specific to point-to-point messaging examples."""

    message: str | None = Field(
        None,
        description="Message to send (activates sender mode)",
    )

    iterations: int = Field(
        default=10,
        description="Number of request/reply cycles in sender mode",
        ge=1,
    )


class ServerConfig(BaseSettings):
    """Configuration for SLIM server."""

    model_config = SettingsConfigDict(
        env_prefix="SLIM_",
        env_file=".env",
        env_file_encoding="utf-8",
        case_sensitive=False,
        extra="ignore",
    )

    slim: str = Field(
        default="127.0.0.1:12345",
        description="SLIM server address (host:port)",
    )

    enable_opentelemetry: bool = Field(
        default=False,
        description="Enable OpenTelemetry tracing",
    )

    @classmethod
    def from_args(cls, **kwargs: Any) -> "ServerConfig":
        """Create config from keyword arguments."""
        filtered = {k: v for k, v in kwargs.items() if v is not None}
        return cls(**filtered)


def load_config_with_cli_override(
    config_class: type[BaseSettings], cli_args: dict[str, Any]
) -> BaseSettings:
    """
    Load configuration with CLI argument override priority.

    Priority order (highest to lowest):
    1. CLI arguments (non-None values)
    2. Environment variables
    3. Config file (if SLIM_CONFIG_FILE env var is set)
    4. Defaults

    Args:
        config_class: The Pydantic settings class to instantiate
        cli_args: Dictionary of CLI arguments

    Returns:
        Configured settings instance
    """
    # Check if config file is specified
    config_file = os.getenv("SLIM_CONFIG_FILE")

    if config_file and hasattr(config_class, "from_file"):
        # Load from file first
        config = config_class.from_file(config_file)
        # Override with CLI args
        if cli_args:
            filtered_cli = {k: v for k, v in cli_args.items() if v is not None}
            config = config_class(**{**config.model_dump(), **filtered_cli})
    else:
        # Load from env vars and CLI args
        filtered_cli = {k: v for k, v in cli_args.items() if v is not None}
        config = config_class(**filtered_cli)

    return config
