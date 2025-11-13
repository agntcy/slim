# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import pytest
import slim_bindings

from slimrpc import common


def test_split_id_success() -> None:
    result = common.split_id("org/ns/app")

    assert isinstance(result, slim_bindings.PyName)
    assert result.components_strings() == ["org", "ns", "app"]


def test_split_id_invalid_format() -> None:
    with pytest.raises(ValueError):
        common.split_id("org/ns")


def test_method_to_pyname_builds_subscription_name() -> None:
    name = slim_bindings.PyName("org", "ns", "app")

    result = common.method_to_pyname(name, "service", "method")

    assert isinstance(result, slim_bindings.PyName)
    assert result.components_strings() == ["org", "ns", "app-service-method"]


def test_service_and_method_to_pyname_parses_path(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    called_with: list[tuple[object, str, str]] = []

    def fake_method_to_pyname(name: object, service: str, method: str) -> str:
        called_with.append((name, service, method))
        return "sentinel"

    monkeypatch.setattr(common, "method_to_pyname", fake_method_to_pyname)

    result = common.service_and_method_to_pyname("pyname", "/svc/method")

    assert called_with == [("pyname", "svc", "method")]
    assert result == "sentinel"


def test_handler_name_to_pyname_delegates(monkeypatch: pytest.MonkeyPatch) -> None:
    called_with: list[tuple[object, str, str]] = []

    def fake_method_to_pyname(name: object, service: str, method: str) -> str:
        called_with.append((name, service, method))
        return "handler"

    monkeypatch.setattr(common, "method_to_pyname", fake_method_to_pyname)

    result = common.handler_name_to_pyname("pyname", "svc", "method")

    assert called_with == [("pyname", "svc", "method")]
    assert result == "handler"


def test_shared_secret_identity_creates_provider_and_verifier() -> None:
    provider, verifier = common.shared_secret_identity("identity", "secret")

    assert isinstance(provider, slim_bindings.PyIdentityProvider)
    assert isinstance(verifier, slim_bindings.PyIdentityVerifier)


def test_app_config_identity_pyname_uses_split_id() -> None:
    config = common.SLIMAppConfig(
        identity="org/ns/app",
        slim_client_config={"endpoint": "example"},
        shared_secret="secret",
    )

    result = config.identity_pyname()

    assert isinstance(result, slim_bindings.PyName)
    assert result.components_strings() == ["org", "ns", "app"]
