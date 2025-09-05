# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from dataclasses import dataclass

from slim_bindings import PySessionInfo


@dataclass
class Context:
    """
    Context for RPC calls.
    """

    session_id: int
    source_name: str
    destination_name: str
    payload_type: str
    metadata: dict[str, str] | None = None

    @classmethod
    def from_sessioninfo(cls, session_info: PySessionInfo) -> "Context":
        """
        Create a Context from session information.
        """
        return cls(
            session_id=session_info.id,
            source_name=str(session_info.source_name),
            destination_name=str(session_info.destination_name),
            payload_type=session_info.payload_type,
            metadata=session_info.metadata,
        )
