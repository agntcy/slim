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

    # Create RPC
    rpc = Rpc(
        name="example_rpc",
        handler=lambda x: f"Hello, {x}!",
        request_deserializer= lambda x: x.decode('utf-8'),
        response_serializer=lambda x: x.encode('utf-8'),
    )
    server.register_rpc(rpc)

    try:
        asyncio.run(server.run())
    except KeyboardInterrupt:
        print("Server interrupted by user.")
