# Copyright AGNTCY Contributors
# SPDX-License-Identifier: Apache-2.0

import pytest
from common import create_slim

from slim_bindings import (
    PyName,
    PySessionConfiguration,
)


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12975"], indirect=True)
async def test_session_metadata_merge_roundtrip(server):
    sender_name = PyName("org", "ns", "sender")
    receiver_name = PyName("org", "ns", "receiver")
    sender = await create_slim(sender_name, "secret")
    receiver = await create_slim(receiver_name, "secret")

    _ = await sender.connect(
        {"endpoint": "http://127.0.0.1:12975", "tls": {"insecure": True}}
    )

    _ = await receiver.connect(
        {"endpoint": "http://127.0.0.1:12975", "tls": {"insecure": True}}
    )

    await sender.set_route(receiver_name)

    metadata = {"a": "1", "k": "session"}

    # Create unicast session
    sess_cfg = PySessionConfiguration.Unicast(receiver_name, metadata=metadata)
    session_sender = await sender.create_session(sess_cfg)

    await session_sender.publish(b"hello")

    # receive session in receiver
    session_receiver = await receiver.listen_for_session()

    # make sure the received metadata matches
    session_metadata = session_receiver.metadata
    for k, v in metadata.items():
        assert v == session_metadata[k]
