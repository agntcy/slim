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

    # Create unicast session
    sess_cfg = PySessionConfiguration.Unicast()
    session_sender = await sender.create_session(sess_cfg)

    # Set session-level metadata
    metadata = {"a": "1", "k": "session"}
    session_sender.set_local_metadata(metadata)

    metadata_override = {"k": "override", "b": "2"}
    await session_sender.publish(b"hello", receiver_name, metadata=metadata_override)

    # receive session in receiver
    session_receiver = await receiver.listen_for_session()

    # make sure the received metadata matches
    session_metadata = session_receiver.received_metadata
    for k, v in metadata.items():
        assert v == session_metadata[k]

    # Now let's receive the message
    ctx, _payload = await session_receiver.get_message()

    # make sure the metadata in the ctx is overridden
    message_metadata = ctx.metadata
    for k, v in metadata_override.items():
        assert v == message_metadata[k]

    # Lets publish now a message without overriding the metadata
    await session_sender.publish(b"hello2", receiver_name)

    # receive it
    ctx, _payload = await session_receiver.get_message()

    # make sure the metadata in the ctx is not overridden
    message_metadata = ctx.metadata
    for k, v in metadata.items():
        assert v == message_metadata[k]
