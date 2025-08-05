# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

from collections.abc import Awaitable


class Rpc:
    """
    Base class for RPC object. It holds
    """

    def __init__(
        self,
        method_name: str,
        handler: Awaitable,  # Or Callable?
        request_deserializer: callable,
        response_serializer: callable,
        service_name: str | None = None,
        request_streaming: bool = False,
        response_streaming: bool = False,
    ):
        self.service_name = service_name
        self.method_name = method_name
        self.handler = handler
        self.request_deserializer = request_deserializer
        self.response_serializer = response_serializer
        self.request_streaming = request_streaming
        self.response_streaming = response_streaming
