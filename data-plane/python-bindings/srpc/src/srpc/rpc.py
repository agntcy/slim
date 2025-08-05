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
        request_serializer: callable,
        response_serializer: callable,
    ):
        self.name = name
        self.handler = handler
        self.request_serializer = request_serializer
        self.response_serializer = response_serializer
