# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from dataclasses import dataclass

import slim_bindings


@dataclass
class SessionContext:
    """
    Context for RPC calls based on session information.
    """

    session_id: int
    source_name: str
    destination_name: str
    metadata: dict[str, str] | None = None

    @classmethod
    def from_session(cls, session: slim_bindings.Session) -> "SessionContext":
        """
        Create a SessionContext from session.
        """
        return cls(
            session_id=session.id,
            source_name=str(session.src),
            destination_name=str(session.dst),
            metadata=session.metadata,
        )


@dataclass
class MessageContext:
    """
    Context for RPC calls based on message information.
    """

    source_name: str
    destination_name: str
    payload_type: str
    metadata: dict[str, str] | None = None

    @classmethod
    def from_message_context(
        cls, msg_ctx: slim_bindings.MessageContext
    ) -> "MessageContext":
        """
        Create a MessageContext from message context.
        """
        return cls(
            source_name=str(msg_ctx.source_name),
            destination_name=str(msg_ctx.destination_name),
            payload_type=msg_ctx.payload_type,
            metadata=msg_ctx.metadata,
        )


# Keep Context as an alias for backward compatibility
Context = SessionContext
