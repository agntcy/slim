# Copyright AGNTCY Contributors
# SPDX-License-Identifier: Apache-2.0

"""
Test: session metadata propagation (round-trip).

Purpose:
  Validate that metadata attached to a Unicast session configuration on the
  initiating side (sender) is visible with identical key/value pairs on the
  receiving side once the session is established.

What is covered:
  * Construction of a Unicast PySessionConfiguration with custom metadata.
  * Session creation by the sender and automatic session notification for receiver.
  * Verification that all metadata entries appear unchanged on the receiver's
    session context (session_receiver.metadata).

Not supported:
  * Mutating metadata after session establishment.

Pass criteria:
  All key/value pairs inserted in the initiating configuration must appear
  exactly once and match on the receiver side.
"""

import pytest
from common import create_slim

from slim_bindings import (
    PyName,
    PySessionConfiguration,
)


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12975"], indirect=True)
async def test_session_metadata_merge_roundtrip(server):
    """Ensure session metadata provided at Unicast session creation is preserved end-to-end.

    Flow:
      1. Create sender & receiver Slim instances.
      2. Sender connects, sets a route to receiver.
      3. Sender creates a Unicast session with metadata.
      4. Sender publishes a message to trigger session establishment on receiver.
      5. Receiver listens for the new session and inspects metadata.
      6. Assert every original key/value is present and unchanged.

    Assertions:
      For each (k, v) in initial metadata: receiver.metadata[k] == v.
    """
    # Define identities
    sender_name = PyName("org", "ns", "sender")
    receiver_name = PyName("org", "ns", "receiver")

    # Instantiate Slim instances with shared-secret auth
    sender = await create_slim(sender_name, "secret")
    receiver = await create_slim(receiver_name, "secret")

    # Connect both to the test server endpoint
    _ = await sender.connect(
        {"endpoint": "http://127.0.0.1:12975", "tls": {"insecure": True}}
    )
    _ = await receiver.connect(
        {"endpoint": "http://127.0.0.1:12975", "tls": {"insecure": True}}
    )

    # Establish routing so sender can reach receiver by name
    await sender.set_route(receiver_name)

    # Metadata we want to propagate with the session creation
    metadata = {"a": "1", "k": "session"}

    # Create unicast session
    sess_cfg = PySessionConfiguration.Unicast(receiver_name, metadata=metadata)
    session_sender = await sender.create_session(sess_cfg)

    await session_sender.publish(b"hello")

    # Receiver obtains the new session context
    session_receiver = await receiver.listen_for_session()

    # Extract and validate metadata
    session_metadata = session_receiver.metadata
    for k, v in metadata.items():
        assert v == session_metadata[k], (
            f"Metadata mismatch for key '{k}': {v} != {session_metadata.get(k)}"
        )
