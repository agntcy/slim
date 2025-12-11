# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import logging
from collections.abc import AsyncGenerator
from dataclasses import dataclass
from typing import Callable

import slim_bindings
import slimrpc
from a2a.client.client import ClientConfig as A2AClientConfig
from a2a.client.middleware import ClientCallContext, ClientCallInterceptor
from a2a.client.transports.base import ClientTransport
from a2a.extensions.common import HTTP_EXTENSION_HEADER
from a2a.grpc import a2a_pb2
from a2a.types import (
    AgentCard,
    GetTaskPushNotificationConfigParams,
    Message,
    MessageSendParams,
    Task,
    TaskArtifactUpdateEvent,
    TaskIdParams,
    TaskPushNotificationConfig,
    TaskQueryParams,
    TaskStatusUpdateEvent,
)
from a2a.utils import proto_utils
from a2a.utils.telemetry import SpanKind, trace_class

from slima2a.types import a2a_pb2_slimrpc

logger = logging.getLogger(__name__)


def slimrpc_channel_factory(
    local_app: slim_bindings.Slim,
) -> Callable[[str], slimrpc.Channel]:
    def factory(remote: str) -> slimrpc.Channel:
        return slimrpc.Channel(local_app=local_app, remote=remote)

    return factory


@dataclass
class ClientConfig(A2AClientConfig):
    slimrpc_channel_factory: Callable[[str], slimrpc.Channel] | None = None


@trace_class(kind=SpanKind.CLIENT)
class SRPCTransport(ClientTransport):
    """A gRPC transport for the A2A client."""

    def __init__(
        self,
        channel: slimrpc.Channel,
        agent_card: AgentCard | None,
    ) -> None:
        """Initializes the GrpcTransport."""
        self.agent_card = agent_card
        self.channel = channel
        self.stub = a2a_pb2_slimrpc.A2AServiceStub(channel)
        self._needs_extended_card = (
            agent_card.supports_authenticated_extended_card if agent_card else True
        )

    @classmethod
    def create(
        cls,
        card: AgentCard,
        url: str,
        config: ClientConfig,
        interceptors: list[ClientCallInterceptor],
    ) -> "SRPCTransport":
        """Creates a gRPC transport for the A2A client."""
        if config.slimrpc_channel_factory is None:
            raise ValueError("slimrpc_channel_factory is required when using sRPC")
        channel = config.slimrpc_channel_factory(url)
        return cls(
            channel,
            card,
        )

    async def send_message(
        self,
        request: MessageSendParams,
        *,
        context: ClientCallContext | None = None,
        extensions: list[str] | None = None,
    ) -> Task | Message:
        """Sends a non-streaming message request to the agent."""
        metadata = {}
        if extensions:
            metadata[HTTP_EXTENSION_HEADER] = ",".join(extensions)
        response = await self.stub.SendMessage(
            a2a_pb2.SendMessageRequest(
                request=proto_utils.ToProto.message(request.message),
                configuration=proto_utils.ToProto.message_send_configuration(
                    request.configuration
                ),
                metadata=proto_utils.ToProto.metadata(request.metadata),
            ),
            metadata=metadata,
        )
        if response.HasField("task"):
            return proto_utils.FromProto.task(response.task)
        return proto_utils.FromProto.message(response.msg)

    async def send_message_streaming(
        self,
        request: MessageSendParams,
        *,
        context: ClientCallContext | None = None,
        extensions: list[str] | None = None,
    ) -> AsyncGenerator[
        Message | Task | TaskStatusUpdateEvent | TaskArtifactUpdateEvent, None
    ]:
        """Sends a streaming message request to the agent and yields responses as they arrive."""
        metadata = {}
        if extensions:
            metadata[HTTP_EXTENSION_HEADER] = ",".join(extensions)
        stream = self.stub.SendStreamingMessage(
            a2a_pb2.SendMessageRequest(
                request=proto_utils.ToProto.message(request.message),
                configuration=proto_utils.ToProto.message_send_configuration(
                    request.configuration
                ),
                metadata=proto_utils.ToProto.metadata(request.metadata),
            ),
            metadata=metadata,
        )
        async for response in stream:
            yield proto_utils.FromProto.stream_response(response)

    async def resubscribe(
        self,
        request: TaskIdParams,
        *,
        context: ClientCallContext | None = None,
        extensions: list[str] | None = None,
    ) -> AsyncGenerator[
        Task | Message | TaskStatusUpdateEvent | TaskArtifactUpdateEvent, None
    ]:
        """Reconnects to get task updates."""
        metadata = {}
        if extensions:
            metadata[HTTP_EXTENSION_HEADER] = ",".join(extensions)
        stream = self.stub.TaskSubscription(
            a2a_pb2.TaskSubscriptionRequest(name=f"tasks/{request.id}"),
            metadata=metadata,
        )
        async for response in stream:
            yield proto_utils.FromProto.stream_response(response)

    async def get_task(
        self,
        request: TaskQueryParams,
        *,
        context: ClientCallContext | None = None,
        extensions: list[str] | None = None,
    ) -> Task:
        """Retrieves the current state and history of a specific task."""
        metadata = {}
        if extensions:
            metadata[HTTP_EXTENSION_HEADER] = ",".join(extensions)
        task = await self.stub.GetTask(
            a2a_pb2.GetTaskRequest(name=f"tasks/{request.id}"), metadata=metadata
        )
        return proto_utils.FromProto.task(task)

    async def cancel_task(
        self,
        request: TaskIdParams,
        *,
        context: ClientCallContext | None = None,
        extensions: list[str] | None = None,
    ) -> Task:
        """Requests the agent to cancel a specific task."""
        metadata = {}
        if extensions:
            metadata[HTTP_EXTENSION_HEADER] = ",".join(extensions)
        task = await self.stub.CancelTask(
            a2a_pb2.CancelTaskRequest(name=f"tasks/{request.id}"), metadata=metadata
        )
        return proto_utils.FromProto.task(task)

    async def set_task_callback(
        self,
        request: TaskPushNotificationConfig,
        *,
        context: ClientCallContext | None = None,
        extensions: list[str] | None = None,
    ) -> TaskPushNotificationConfig:
        """Sets or updates the push notification configuration for a specific task."""
        metadata = {}
        if extensions:
            metadata[HTTP_EXTENSION_HEADER] = ",".join(extensions)
        config = await self.stub.CreateTaskPushNotificationConfig(
            a2a_pb2.CreateTaskPushNotificationConfigRequest(
                parent=f"tasks/{request.task_id}",
                config_id=request.push_notification_config.id,
                config=proto_utils.ToProto.task_push_notification_config(request),
            ),
            metadata=metadata,
        )
        return proto_utils.FromProto.task_push_notification_config(config)

    async def get_task_callback(
        self,
        request: GetTaskPushNotificationConfigParams,
        *,
        context: ClientCallContext | None = None,
        extensions: list[str] | None = None,
    ) -> TaskPushNotificationConfig:
        """Retrieves the push notification configuration for a specific task."""
        metadata = {}
        if extensions:
            metadata[HTTP_EXTENSION_HEADER] = ",".join(extensions)
        config = await self.stub.GetTaskPushNotificationConfig(
            a2a_pb2.GetTaskPushNotificationConfigRequest(
                name=f"tasks/{request.id}/pushNotificationConfigs/{request.push_notification_config_id}",
            ),
            metadata=metadata,
        )
        return proto_utils.FromProto.task_push_notification_config(config)

    async def get_card(
        self,
        *,
        context: ClientCallContext | None = None,
        extensions: list[str] | None = None,
    ) -> AgentCard:
        """Retrieves the agent's card."""
        card = self.agent_card
        if card and not self._needs_extended_card:
            return card
        if card is None and not self._needs_extended_card:
            raise ValueError("Agent card is not available.")

        metadata = {}
        if extensions:
            metadata[HTTP_EXTENSION_HEADER] = ",".join(extensions)
        card_pb = await self.stub.GetAgentCard(
            a2a_pb2.GetAgentCardRequest(),
            metadata=metadata,
        )
        card = proto_utils.FromProto.agent_card(card_pb)
        self.agent_card = card
        self._needs_extended_card = False
        return card

    async def close(self) -> None:
        """Closes the transport and releases any resources."""
        pass
