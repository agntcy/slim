# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import pytest

from slimrpc import channel


def test_compute_deadline(monkeypatch: pytest.MonkeyPatch) -> None:
    monkeypatch.setattr(channel.time, "time", lambda: 100.0)

    assert channel._compute_deadline(5) == 105.0  # noqa: SLF001
