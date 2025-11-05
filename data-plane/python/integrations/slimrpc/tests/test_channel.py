# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from slimrpc import channel


def test_compute_deadline(monkeypatch) -> None:
    monkeypatch.setattr(channel.time, "time", lambda: 100.0)

    assert channel._compute_deadline(5) == 105.0

