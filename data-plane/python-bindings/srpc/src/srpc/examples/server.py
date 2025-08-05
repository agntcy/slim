import asyncio

from ..server import Server
from ..rpc import Rpc


def create_server(
    local: str,
    slim: dict,
    enable_opentelemetry: bool = False,
    shared_secret: str | None = None,
):
    """
    Create a new SRPC server instance.
    """
    server = Server(
        local=local,
        slim=slim,
        enable_opentelemetry=enable_opentelemetry,
        shared_secret=shared_secret,
    )

    return server


def main():
    server = create_server(
        local="agntcy/grpc/server",
        slim={
            "endpoint": "http://localhost:46357",
            "tls": {
                "insecure": True,
            },
        },
        enable_opentelemetry=False,
        shared_secret="my_shared_secret",
    )

    # Create RPCs
    server.register_method_handlers(
        "example_service",
        {
            "example_method_1": Rpc(
                method_name="example_method_1",
                handler=lambda request: f"Hello, {request}!",
                request_deserializer=lambda x: x.decode("utf-8"),
                response_serializer=lambda x: x.encode("utf-8"),
            ),
            "example_method_2": Rpc(
                method_name="example_method_2",
                handler=lambda request: f"Hello, {request}!",
                request_deserializer=lambda x: x.decode("utf-8"),
                response_serializer=lambda x: x.encode("utf-8"),
            ),
        },
    )

    try:
        asyncio.run(server.run())
    except KeyboardInterrupt:
        print("Server interrupted by user.")
