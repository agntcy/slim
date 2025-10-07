# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

from ._slim_bindings import (
    PyMessageContext,
    PyName,
    PyService,
    PySessionConfiguration,
    PySessionContext,
    PySessionType,
)
from ._slim_bindings import delete_session as _delete_session
from ._slim_bindings import (
    get_message as _get_message,
)
from ._slim_bindings import (
    invite as _invite,
)
from ._slim_bindings import (
    publish as _publish,
)
from ._slim_bindings import (
    remove as _remove,
)
from ._slim_bindings import (
    set_default_session_config as _set_default_session_config,  # noqa:F401
)


class PySession:
    """High level Python wrapper around a `PySessionContext`.

    This object provides a Pythonic façade over the lower-level Rust session
    context. It retains a reference to the owning `PyService` so the existing
    service-level binding functions (publish, invite, remove, get_message,
    delete_session) can be invoked without duplicating logic on the Rust side.

    Threading / Concurrency:
        The methods are all async (where network / I/O is involved) and are
        safe to await concurrently.

    Lifecycle:
        A `PySession` is typically obtained from `Slim.create_session(...)`
        or `Slim.listen_for_session(...)`. Call `delete()`to release
        server-side resources.

    Attributes (properties):
        id (int): Unique numeric session identifier.
        metadata (dict[str, str]): Free-form key/value metadata attached
            to the current session configuration.
        session_type (PySessionType): P2P / GROUP classification.
        session_config (PySessionConfiguration): Current effective configuration.
        src (PyName): Source name (creator / initiator of the session).
        dst (PyName): Destination name (p2p), Channel name (group)
    """

    def __init__(self, svc: PyService, ctx: PySessionContext):
        self._svc = svc
        self._ctx = ctx

    @property
    def id(self) -> int:
        """Return the unique numeric identifier for this session."""
        return self._ctx.id  # exposed by PySessionContext

    @property
    def metadata(self) -> dict[str, str]:
        """Return a copy of the session metadata mapping (string keys/values)."""
        return self._ctx.metadata

    @property
    def session_type(self) -> PySessionType:
        """Return the type of this session (P2P / GROUP)."""
        return self._ctx.session_type

    @property
    def session_config(self) -> PySessionConfiguration:
        """Return the current effective session configuration enum variant."""
        return self._ctx.session_config

    @property
    def src(self) -> PyName:
        """Return the source name of this session."""
        return self._ctx.src

    @property
    def dst(self) -> PyName:
        """Return the destination name"""
        return self._ctx.dst

    def set_session_config(self, config: PySessionConfiguration) -> None:
        """Replace the current session configuration.

        Args:
            config: A new `PySessionConfiguration` variant.

        Raises:
            RuntimeError (wrapped from Rust) if the configuration change
            is invalid or the session is already closed.
        """
        self._ctx.set_session_config(config)

    async def publish(
        self,
        msg: bytes,
        payload_type: str | None = None,
        metadata: dict | None = None,
    ) -> None:
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

        await _publish(
            self._svc,
            self._ctx,
            1,
            msg,
            message_ctx=None,
            name=None,
            payload_type=payload_type,
            metadata=metadata,
        )

    async def publish_to(
        self,
        message_ctx: PyMessageContext,
        msg: bytes,
        payload_type: str | None = None,
        metadata: dict | None = None,
    ):
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

        await _publish(
            self._svc,
            self._ctx,
            1,
            msg,
            message_ctx=message_ctx,
            payload_type=payload_type,
            metadata=metadata,
        )

    async def invite(self, name: PyName) -> None:
        """Invite (add) a participant to this session. Only works for GROUP.

        Args:
            name: PyName of the participant to invite.

        Raises:
            RuntimeError (wrapped) if the invite fails.
        """
        await _invite(self._svc, self._ctx, name)

    async def remove(self, name: PyName) -> None:
        """Remove (eject) a participant from this session. Only works for GROUP.

        Args:
            name: PyName of the participant to remove.

        Raises:
            RuntimeError (wrapped) if removal fails.
        """
        await _remove(self._svc, self._ctx, name)

    async def get_message(
        self,
    ) -> tuple[PyMessageContext, bytes]:  # PyMessageContext, blob
        """Wait for and return the next inbound message.

        Returns:
            (PyMessageContext, bytes): A tuple containing the message context
            (routing / origin metadata) and the raw payload bytes.

        Raises:
            RuntimeError (wrapped) if the session is closed or receive fails.
        """
        return await _get_message(self._svc, self._ctx)

    async def delete(self) -> None:
        """Terminate the session and release associated resources."""
        await _delete_session(self._svc, self._ctx)

    # Convenience aliases
    async def recv(self) -> tuple[PyMessageContext, bytes]:
        """Alias for `get_message()`."""
        return await self.get_message()


__all__ = [
    "PySession",
]
