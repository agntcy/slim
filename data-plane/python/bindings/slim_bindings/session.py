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

    This wrapper keeps a reference to the owning `PyService` so that all
    existing service-level pyfunctions (publish, invite, remove, get_message,
    delete_session) can be used without changing the Rust bindings.
    """

    def __init__(self, svc: PyService, ctx: PySessionContext):
        self._svc = svc
        self._ctx = ctx

    @property
    def id(self) -> int:
        return self._ctx.id  # exposed by PySessionContext

    @property
    def metadata(self) -> dict[str, str]:
        return self._ctx.metadata

    @property
    def session_type(self) -> PySessionType:
        return self._ctx.session_type

    @property
    def session_config(self) -> PySessionConfiguration:
        return self._ctx.session_config

    @property
    def src(self) -> PyName:
        return self._ctx.src

    @property
    def dst(self) -> PyName | None:
        return self._ctx.dst

    def set_session_config(self, config: PySessionConfiguration) -> None:
        self._ctx.set_session_config(config)

    def _destination_name(self) -> PyName:
        """
        Return the destination name implied by the current session configuration.
        Unicast -> unicast_name
        Multicast -> topic
        Anycast has no fixed destination (raises).
        """
        cfg = self._ctx.session_config
        if hasattr(cfg, "unicast_name"):  # Unicast variant
            return cfg.unicast_name
        if hasattr(cfg, "topic"):  # Multicast variant
            return cfg.topic
        raise RuntimeError("ANYCAST session has no fixed destination")

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

        if self._ctx.session_type == PySessionType.ANYCAST:
            raise RuntimeError("unexpected session type: expected UNICAST or MULTICAST")

        dst = self._destination_name()

        await _publish(
            self._svc,
            self._ctx,
            1,
            msg,
            message_ctx=None,
            name=dst,
            payload_type=payload_type,
            metadata=metadata,
        )

    async def publish_with_destination(
        self,
        msg: bytes,
        dest: PyName,
        payload_type: str | None = None,
        metadata: dict | None = None,
    ) -> None:
        """
        Publish a message with a destination name on an existing session.
        This is possible only on Anycast sessions. The function returns an error
        in other cases.

        Args:
            msg (bytes): The message payload to publish.
            dest (PyName): The destination name for the message.
            payload_type (str, optional): The type of the payload, if applicable.
            metadata (dict, optional): Additional metadata to include with the
                message.

        Returns:
            None
        """
        if self._ctx.session_type != PySessionType.ANYCAST:
            raise RuntimeError("unexpected session type: expected ANYCAST")

        await _publish(
            self._svc,
            self._ctx,
            1,
            msg,
            message_ctx=None,
            name=dest,
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
        Publish a message back to the application that sent the original message.
        The source application information is retrieved from the message context and
        session.

        Args:
            message_ctx (PyMessageContext): The context of the original message,
                used to identify the source application.
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
            message_ctx=message_ctx,
            payload_type=payload_type,
            metadata=metadata,
        )

    async def invite(self, name: PyName) -> None:
        await _invite(self._svc, self._ctx, name)

    async def remove(self, name: PyName) -> None:
        await _remove(self._svc, self._ctx, name)

    async def get_message(
        self,
    ) -> tuple[PyMessageContext, bytes]:  # PyMessageContext, blob
        return await _get_message(self._svc, self._ctx)

    async def delete(self) -> None:
        await _delete_session(self._svc, self._ctx)

    # Convenience aliases
    async def recv(self) -> tuple[PyMessageContext, bytes]:
        return await self.get_message()


__all__ = [
    "PySession",
]
