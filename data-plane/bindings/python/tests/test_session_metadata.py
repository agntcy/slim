# Copyright AGNTCY Contributors
# SPDX-License-Identifier: Apache-2.0

"""
Test: session metadata propagation (round-trip).

Purpose:
  Validate that metadata attached to a PointToPoint session configuration on the
  initiating side (sender) is visible with identical key/value pairs on the
  receiving side once the session is established.

What is covered:
  * Construction of a PointToPoint SessionConfiguration with custom metadata.
  * Session creation by the sender and automatic session notification for receiver.
  * Verification that all metadata entries appear unchanged on the receiver's
    session context (session_receiver.metadata).

Not supported:
  * Mutating metadata after session establishment.

Pass criteria:
  All key/value pairs inserted in the initiating configuration must appear
  exactly once and match on the receiver side.
"""

import datetime
import uuid

import pytest

import slim_bindings

LONG_SECRET = "e4aaecb9ae0b23b82086bb8a8633e01fba16ae8d9c1379a613c00838"


@pytest.mark.asyncio
@pytest.mark.parametrize("server", [None], indirect=True)
async def test_session_metadata_merge_roundtrip(server):
    """Ensure session metadata provided at PointToPoint session creation is preserved end-to-end.

    Flow:
      1. Create sender & receiver Slim instances.
      2. Sender connects, sets a route to receiver.
      3. Sender creates a PointToPoint session with metadata.
      4. Sender publishes a message to trigger session establishment on receiver.
      5. Receiver listens for the new session and inspects metadata.
      6. Assert every original key/value is present and unchanged.

    Assertions:
      For each (k, v) in initial metadata: receiver.metadata[k] == v.
    """
    # Generate unique names to avoid collisions
    test_id = str(uuid.uuid4())[:8]
    sender_name = slim_bindings.Name("org", f"test_{test_id}", "sessionsender")
    receiver_name = slim_bindings.Name("org", f"test_{test_id}", "sessionreceiver")

    # Use global service
    svc = server.service

    # Create sender and receiver apps
    sender = svc.create_app_with_secret(sender_name, LONG_SECRET)
    receiver = svc.create_app_with_secret(receiver_name, LONG_SECRET)

    # Metadata we want to propagate with the session creation
    metadata = {"a": "1", "k": "session"}

    # Create PointToPoint session with metadata
    sess_cfg = slim_bindings.SessionConfig(
        session_type=slim_bindings.SessionType.POINT_TO_POINT,
        enable_mls=False,
        max_retries=5,
        interval=datetime.timedelta(seconds=1),
        metadata=metadata,
    )
    # Create session (returns SessionWithCompletion)
    session_context_sender = await sender.create_session_async(sess_cfg, receiver_name)
    await session_context_sender.completion.wait_async()
    session_sender = session_context_sender.session

    # Publish a message to trigger session on receiver side
    await session_sender.publish_async(b"hello", None, None)

    # Receiver obtains the new session context
    session_context_receiver = await receiver.listen_for_session_async(
        timeout=datetime.timedelta(seconds=10)
    )

    # Extract and validate metadata
    session_metadata = session_context_receiver.metadata()
    for k, v in metadata.items():
        assert v == session_metadata.get(k), (
            f"Metadata mismatch for key '{k}': {v} != {session_metadata.get(k)}"
        )

    # Delete sessions
    handle = await sender.delete_session_async(session_sender)
    await handle.wait_async()
