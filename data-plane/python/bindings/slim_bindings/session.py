# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

import datetime
import typing

from ._slim_bindings import (
    MessageContext,
    Name,
    SessionConfiguration,
    SessionContext,
    SessionType,
)


class Session:
    """High level Python wrapper around a `SessionContext`.

    This object provides a Pythonic façade over the lower-level Rust session
    context. It retains a reference to the owning `App` so the existing
    service-level binding functions (publish, invite, remove, get_message,
    delete_session) can be invoked without duplicating logic on the Rust side.

    Threading / Concurrency:
        The methods are all async (where network / I/O is involved) and are
        safe to await concurrently.

    Lifecycle:
        A `Session` is typically obtained from `Slim.create_session(...)`
        or `Slim.listen_for_session(...)`. Call `delete()`to release
        server-side resources.

    Attributes (properties):
        id (int): Unique numeric session identifier.
        metadata (dict[str, str]): Free-form key/value metadata attached
            to the current session configuration.
        session_type (SessionType): PointToPoint / Group classification.
        session_config (SessionConfiguration): Current effective configuration.
        src (Name): Source name (creator / initiator of the session).
        dst (Name): Destination name (PointToPoint), Channel name (group)
    """

    def __init__(self, ctx: SessionContext):
        self._ctx = ctx

    @property
    def id(self) -> int:
        """Return the unique numeric identifier for this session."""
        return self._ctx.id  # exposed by SessionContext

    @property
    def metadata(self) -> dict[str, str]:
        """Return a copy of the session metadata mapping (string keys/values)."""
        return self._ctx.metadata

    @property
    def session_type(self) -> SessionType:
        """Return the type of this session (PointToPoint / Group)."""
        return self._ctx.session_type

    @property
    def session_config(self) -> SessionConfiguration:
        """Return the current effective session configuration enum variant."""
        return self._ctx.session_config

    @property
    def src(self) -> Name:
        """Return the source name of this session."""
        return self._ctx.src

    @property
    def dst(self) -> Name | None:
        """Return the destination name"""
        return self._ctx.dst

    def publish(
        self,
        msg: bytes,
        payload_type: str | None = None,
        metadata: dict | None = None,
    ) -> typing.Any:
        """
        Publish a message on the current session.

        Args:
            msg (bytes): The message payload to publish.
            payload_type (str, optional): The type of the payload, if applicable.
            metadata (dict, optional): Additional metadata to include with the
                message.

        Returns:
            None
        """

        return self._ctx.publish(
            1,
            msg,
            message_ctx=None,
            name=None,
            payload_type=payload_type,
            metadata=metadata,
        )

    async def publish_to(
        self,
        message_ctx: MessageContext,
        msg: bytes,
        payload_type: str | None = None,
        metadata: dict | None = None,
    ) -> typing.Any:
        """
        Publish a message directly back to the originator associated with the
        supplied `message_ctx` (reply semantics).

        Args:
            message_ctx: The context previously received with a message from
                `get_message()` / `recv()`. Provides addressing info.
            msg: Raw bytes payload to send as the reply.
            payload_type: Optional content-type / discriminator.
            metadata: Optional message-scoped metadata.

        Notes:
            The explicit `dest` parameter is not required because the routing
            information is derived from `message_ctx`.

        Raises:
            RuntimeError (wrapped) if sending fails or the session is closed.
        """

        return self._ctx.publish_to(
            message_ctx,
            msg,
            payload_type=payload_type,
            metadata=metadata,
        )

    async def invite(self, name: Name) -> typing.Any:
        """Invite (add) a participant to this session. Only works for Group.

        Args:
            name: Name of the participant to invite.

        Raises:
            RuntimeError (wrapped) if the invite fails.
        """
        return await self._ctx.invite(name)

    async def remove(self, name: Name) -> typing.Any:
        """Remove (eject) a participant from this session. Only works for Group.

        Args:
            name: Name of the participant to remove.

        Raises:
            RuntimeError (wrapped) if removal fails.
        """
        return await self._ctx.remove(name)

    async def get_message(
        self, timeout: datetime.timedelta | None = None
    ) -> tuple[MessageContext, bytes]:  # MessageContext, blob
        """Wait for and return the next inbound message.

        Returns:
            (MessageContext, bytes): A tuple containing the message context
            (routing / origin metadata) and the raw payload bytes.

        Raises:
            RuntimeError (wrapped) if the session is closed or receive fails.
        """
        return await self._ctx.get_message(timeout)


__all__ = [
    "Session",
]
