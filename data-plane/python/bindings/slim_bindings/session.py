# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from __future__ import annotations

from ._slim_bindings import (
    PyMessageContext,
    PyName,
    PyService,
    PySessionConfiguration,
    PySessionContext,
)
from ._slim_bindings import delete_session as _delete_session
from ._slim_bindings import (
    get_message as _get_message,
)
from ._slim_bindings import (
    invite as _invite,
)
from ._slim_bindings import (
    remove as _remove,
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

    def get_session_config(self) -> PySessionConfiguration:
        return self._ctx.get_session_config()

    def set_session_config(self, config: PySessionConfiguration) -> None:
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
            name=dest,
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
