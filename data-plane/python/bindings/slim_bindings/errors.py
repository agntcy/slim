# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from asyncio import TimeoutError


class SLIMTimeoutError(TimeoutError):
    """
    Exception raised for SLIM timeout errors.

    This exception is raised when an operation in an SLIM session times out.
    It encapsulates detailed information about the timeout event, including the
    ID of the message that caused the timeout and the session identifier. An
    optional underlying exception can also be provided to offer additional context.

    Attributes:
        message_id (int): The identifier associated with the message triggering the timeout.
        session_id (int): The identifier of the session where the timeout occurred.
        message (str): A brief description of the timeout error.
        original_exception (Exception, optional): The underlying exception that caused the timeout, if any.

    The string representation of the exception (via __str__) returns a full message that
    includes the custom message, session ID, and message ID, as well as details of the
    original exception (if present). This provides a richer context when the exception is logged
    or printed.
    """

    def __init__(
        self,
        message_id: int,
        session_id: int,
        message: str = "SLIM timeout error",
        original_exception: Exception | None = None,
    ):
        self.message_id = message_id
        self.session_id = session_id
        self.message = message
        self.original_exception = original_exception
        full_message = f"{message} for session {session_id} and message {message_id}"
        if original_exception:
            full_message = f"{full_message}. Caused by: {original_exception!r}"
        super().__init__(full_message)

    def __str__(self):
        return self.args[0]

    def __repr__(self):
        return (
            f"{self.__class__.__name__}(session_id={self.session_id!r}, "
            f"message_id={self.message_id!r}, "
            f"message={self.message!r}, original_exception={self.original_exception!r})"
        )
