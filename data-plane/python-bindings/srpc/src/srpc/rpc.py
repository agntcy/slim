# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0


class Rpc:
    """
    Base class for RPC object. It holds
    """

    def __init__(
        self,
        name: str,
        handler: dict,
        request_deserializer: callable,
        response_serializer: callable,
    ):
        self.name = name
        self.handler = handler
        self.request_deserializer = request_deserializer
        self.response_serializer = response_serializer
