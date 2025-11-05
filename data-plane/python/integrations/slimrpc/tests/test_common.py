# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from types import SimpleNamespace

import pytest

from slimrpc import common


@pytest.fixture
def dummy_pyname_cls():
    class DummyPyName:
        def __init__(self, organization: str, namespace: str, app: str) -> None:
            self.args = (organization, namespace, app)

        def components_strings(self) -> list[str]:
            return list(self.args)

        def __str__(self) -> str:
            return "/".join(self.args)

    return DummyPyName


def test_split_id_success(monkeypatch: pytest.MonkeyPatch, dummy_pyname_cls) -> None:
    monkeypatch.setattr(
        common,
        "slim_bindings",
        SimpleNamespace(PyName=dummy_pyname_cls),
    )

    result = common.split_id("org/ns/app")

    assert isinstance(result, dummy_pyname_cls)
    assert result.args == ("org", "ns", "app")


def test_split_id_invalid_format(monkeypatch: pytest.MonkeyPatch, dummy_pyname_cls) -> None:
    monkeypatch.setattr(
        common,
        "slim_bindings",
        SimpleNamespace(PyName=dummy_pyname_cls),
    )

    with pytest.raises(ValueError):
        common.split_id("org/ns")


def test_method_to_pyname_builds_subscription_name(
    monkeypatch: pytest.MonkeyPatch, dummy_pyname_cls
) -> None:
    monkeypatch.setattr(
        common,
        "slim_bindings",
        SimpleNamespace(PyName=dummy_pyname_cls),
    )

    name = dummy_pyname_cls("org", "ns", "app")

    result = common.method_to_pyname(name, "service", "method")

    assert isinstance(result, dummy_pyname_cls)
    assert result.args == ("org", "ns", "app-service-method")


def test_service_and_method_to_pyname_parses_path(monkeypatch: pytest.MonkeyPatch) -> None:
    called_with: tuple = ()

    def fake_method_to_pyname(name: object, service: str, method: str) -> str:
        nonlocal called_with
        called_with = (name, service, method)
        return "sentinel"

    monkeypatch.setattr(common, "method_to_pyname", fake_method_to_pyname)

    result = common.service_and_method_to_pyname("pyname", "/svc/method")

    assert called_with == ("pyname", "svc", "method")
    assert result == "sentinel"


def test_handler_name_to_pyname_delegates(monkeypatch: pytest.MonkeyPatch) -> None:
    called_with: tuple = ()

    def fake_method_to_pyname(name: object, service: str, method: str) -> str:
        nonlocal called_with
        called_with = (name, service, method)
        return "handler"

    monkeypatch.setattr(common, "method_to_pyname", fake_method_to_pyname)

    result = common.handler_name_to_pyname("pyname", "svc", "method")

    assert called_with == ("pyname", "svc", "method")
    assert result == "handler"


def test_shared_secret_identity_creates_provider_and_verifier(
    monkeypatch: pytest.MonkeyPatch,
) -> None:
    class DummyProvider:
        @staticmethod
        def SharedSecret(identity: str, shared_secret: str) -> tuple[str, str, str]:
            return ("provider", identity, shared_secret)

    class DummyVerifier:
        @staticmethod
        def SharedSecret(identity: str, shared_secret: str) -> tuple[str, str, str]:
            return ("verifier", identity, shared_secret)

    monkeypatch.setattr(
        common,
        "slim_bindings",
        SimpleNamespace(
            PyIdentityProvider=DummyProvider,
            PyIdentityVerifier=DummyVerifier,
        ),
    )

    provider, verifier = common.shared_secret_identity("identity", "secret")

    assert provider == ("provider", "identity", "secret")
    assert verifier == ("verifier", "identity", "secret")


def test_app_config_identity_pyname_uses_split_id(monkeypatch: pytest.MonkeyPatch) -> None:
    captured: dict[str, str] = {}

    def fake_split_id(value: str) -> str:
        captured["identity"] = value
        return "pyname"

    monkeypatch.setattr(common, "split_id", fake_split_id)

    config = common.SLIMAppConfig(
        identity="org/ns/app",
        slim_client_config={"endpoint": "example"},
        shared_secret="secret",
    )

    assert config.identity_pyname() == "pyname"
    assert captured["identity"] == "org/ns/app"

