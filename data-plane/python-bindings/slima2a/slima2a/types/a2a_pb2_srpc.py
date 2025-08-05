"""Client and server classes corresponding to protobuf-defined services."""

from google.protobuf import empty_pb2 as google_dot_protobuf_dot_empty__pb2
from google.rpc import code_pb2 as code__pb2

import srpc
from srpc import rpc as srpc_rpc

from . import a2a_pb2 as a2a__pb2


class A2AServiceStub:
    """A2AService defines the gRPC version of the A2A protocol. This has a slightly
    different shape than the JSONRPC version to better conform to AIP-127,
    where appropriate. The nouns are AgentCard, Message, Task and
    TaskPushNotificationConfig.
    - Messages are not a standard resource so there is no get/delete/update/list
    interface, only a send and stream custom methods.
    - Tasks have a get interface and custom cancel and subscribe methods.
    - TaskPushNotificationConfig are a resource whose parent is a task.
    They have get, list and create methods.
    - AgentCard is a static resource with only a get method.
    fields are not present as they don't comply with AIP rules, and the
    optional history_length on the get task method is not present as it also
    violates AIP-127 and AIP-131.
    """

    def __init__(self, channel):
        """Constructor.

        Args:
            channel: A grpc.Channel.
        """
        self.SendMessage = channel.unary_unary(
            "/a2a.v1.A2AService/SendMessage",
            request_serializer=a2a__pb2.SendMessageRequest.SerializeToString,
            response_deserializer=a2a__pb2.SendMessageResponse.FromString,
            _registered_method=True,
        )
        self.SendStreamingMessage = channel.unary_stream(
            "/a2a.v1.A2AService/SendStreamingMessage",
            request_serializer=a2a__pb2.SendMessageRequest.SerializeToString,
            response_deserializer=a2a__pb2.StreamResponse.FromString,
            _registered_method=True,
        )
        self.GetTask = channel.unary_unary(
            "/a2a.v1.A2AService/GetTask",
            request_serializer=a2a__pb2.GetTaskRequest.SerializeToString,
            response_deserializer=a2a__pb2.Task.FromString,
            _registered_method=True,
        )
        self.CancelTask = channel.unary_unary(
            "/a2a.v1.A2AService/CancelTask",
            request_serializer=a2a__pb2.CancelTaskRequest.SerializeToString,
            response_deserializer=a2a__pb2.Task.FromString,
            _registered_method=True,
        )
        self.TaskSubscription = channel.unary_stream(
            "/a2a.v1.A2AService/TaskSubscription",
            request_serializer=a2a__pb2.TaskSubscriptionRequest.SerializeToString,
            response_deserializer=a2a__pb2.StreamResponse.FromString,
            _registered_method=True,
        )
        self.CreateTaskPushNotificationConfig = channel.unary_unary(
            "/a2a.v1.A2AService/CreateTaskPushNotificationConfig",
            request_serializer=a2a__pb2.CreateTaskPushNotificationConfigRequest.SerializeToString,
            response_deserializer=a2a__pb2.TaskPushNotificationConfig.FromString,
            _registered_method=True,
        )
        self.GetTaskPushNotificationConfig = channel.unary_unary(
            "/a2a.v1.A2AService/GetTaskPushNotificationConfig",
            request_serializer=a2a__pb2.GetTaskPushNotificationConfigRequest.SerializeToString,
            response_deserializer=a2a__pb2.TaskPushNotificationConfig.FromString,
            _registered_method=True,
        )
        self.ListTaskPushNotificationConfig = channel.unary_unary(
            "/a2a.v1.A2AService/ListTaskPushNotificationConfig",
            request_serializer=a2a__pb2.ListTaskPushNotificationConfigRequest.SerializeToString,
            response_deserializer=a2a__pb2.ListTaskPushNotificationConfigResponse.FromString,
            _registered_method=True,
        )
        self.GetAgentCard = channel.unary_unary(
            "/a2a.v1.A2AService/GetAgentCard",
            request_serializer=a2a__pb2.GetAgentCardRequest.SerializeToString,
            response_deserializer=a2a__pb2.AgentCard.FromString,
            _registered_method=True,
        )
        self.DeleteTaskPushNotificationConfig = channel.unary_unary(
            "/a2a.v1.A2AService/DeleteTaskPushNotificationConfig",
            request_serializer=a2a__pb2.DeleteTaskPushNotificationConfigRequest.SerializeToString,
            response_deserializer=google_dot_protobuf_dot_empty__pb2.Empty.FromString,
            _registered_method=True,
        )


class A2AServiceServicer:
    """A2AService defines the gRPC version of the A2A protocol. This has a slightly
    different shape than the JSONRPC version to better conform to AIP-127,
    where appropriate. The nouns are AgentCard, Message, Task and
    TaskPushNotificationConfig.
    - Messages are not a standard resource so there is no get/delete/update/list
    interface, only a send and stream custom methods.
    - Tasks have a get interface and custom cancel and subscribe methods.
    - TaskPushNotificationConfig are a resource whose parent is a task.
    They have get, list and create methods.
    - AgentCard is a static resource with only a get method.
    fields are not present as they don't comply with AIP rules, and the
    optional history_length on the get task method is not present as it also
    violates AIP-127 and AIP-131.
    """

    def SendMessage(self, request, context):
        """Send a message to the agent. This is a blocking call that will return the
        task once it is completed, or a LRO if requested.
        """
        raise srpc_rpc.ErrorResponse(
            code=code__pb2.UNIMPLEMENTED, message="Method not implemented!"
        )

    def SendStreamingMessage(self, request, context):
        """SendStreamingMessage is a streaming call that will return a stream of
        task update events until the Task is in an interrupted or terminal state.
        """
        raise srpc_rpc.ErrorResponse(
            code=code__pb2.UNIMPLEMENTED, message="Method not implemented!"
        )

    def GetTask(self, request, context):
        """Get the current state of a task from the agent."""
        raise srpc_rpc.ErrorResponse(
            code=code__pb2.UNIMPLEMENTED, message="Method not implemented!"
        )

    def CancelTask(self, request, context):
        """Cancel a task from the agent. If supported one should expect no
        more task updates for the task.
        """
        raise srpc_rpc.ErrorResponse(
            code=code__pb2.UNIMPLEMENTED, message="Method not implemented!"
        )

    def TaskSubscription(self, request, context):
        """TaskSubscription is a streaming call that will return a stream of task
        update events. This attaches the stream to an existing in process task.
        If the task is complete the stream will return the completed task (like
        GetTask) and close the stream.
        """
        raise srpc_rpc.ErrorResponse(
            code=code__pb2.UNIMPLEMENTED, message="Method not implemented!"
        )

    def CreateTaskPushNotificationConfig(self, request, context):
        """Set a push notification config for a task."""
        raise srpc_rpc.ErrorResponse(
            code=code__pb2.UNIMPLEMENTED, message="Method not implemented!"
        )

    def GetTaskPushNotificationConfig(self, request, context):
        """Get a push notification config for a task."""
        raise srpc_rpc.ErrorResponse(
            code=code__pb2.UNIMPLEMENTED, message="Method not implemented!"
        )

    def ListTaskPushNotificationConfig(self, request, context):
        """Get a list of push notifications configured for a task."""
        raise srpc_rpc.ErrorResponse(
            code=code__pb2.UNIMPLEMENTED, message="Method not implemented!"
        )

    def GetAgentCard(self, request, context):
        """GetAgentCard returns the agent card for the agent."""
        raise srpc_rpc.ErrorResponse(
            code=code__pb2.UNIMPLEMENTED, message="Method not implemented!"
        )

    def DeleteTaskPushNotificationConfig(self, request, context):
        """Delete a push notification config for a task."""
        raise srpc_rpc.ErrorResponse(
            code=code__pb2.UNIMPLEMENTED, message="Method not implemented!"
        )


def add_A2AServiceServicer_to_server(servicer, server):
    rpc_method_handlers = {
        "SendMessage": srpc.Rpc(
            handler=servicer.SendMessage,
            request_deserializer=a2a__pb2.SendMessageRequest.FromString,
            response_serializer=a2a__pb2.SendMessageResponse.SerializeToString,
            request_streaming=False,
            response_streaming=False,
        ),
        "SendStreamingMessage": srpc.Rpc(
            handler=servicer.SendStreamingMessage,
            request_deserializer=a2a__pb2.SendMessageRequest.FromString,
            response_serializer=a2a__pb2.StreamResponse.SerializeToString,
            request_streaming=False,
            response_streaming=True,
        ),
        "GetTask": srpc.Rpc(
            handler=servicer.GetTask,
            request_deserializer=a2a__pb2.GetTaskRequest.FromString,
            response_serializer=a2a__pb2.Task.SerializeToString,
            request_streaming=False,
            response_streaming=False,
        ),
        "CancelTask": srpc.Rpc(
            handler=servicer.CancelTask,
            request_deserializer=a2a__pb2.CancelTaskRequest.FromString,
            response_serializer=a2a__pb2.Task.SerializeToString,
            request_streaming=False,
            response_streaming=False,
        ),
        "TaskSubscription": srpc.Rpc(
            handler=servicer.TaskSubscription,
            request_deserializer=a2a__pb2.TaskSubscriptionRequest.FromString,
            response_serializer=a2a__pb2.StreamResponse.SerializeToString,
            request_streaming=False,
            response_streaming=True,
        ),
        "CreateTaskPushNotificationConfig": srpc.Rpc(
            handler=servicer.CreateTaskPushNotificationConfig,
            request_deserializer=a2a__pb2.CreateTaskPushNotificationConfigRequest.FromString,
            response_serializer=a2a__pb2.TaskPushNotificationConfig.SerializeToString,
            request_streaming=False,
            response_streaming=False,
        ),
        "GetTaskPushNotificationConfig": srpc.Rpc(
            handler=servicer.GetTaskPushNotificationConfig,
            request_deserializer=a2a__pb2.GetTaskPushNotificationConfigRequest.FromString,
            response_serializer=a2a__pb2.TaskPushNotificationConfig.SerializeToString,
            request_streaming=False,
            response_streaming=False,
        ),
        "ListTaskPushNotificationConfig": srpc.Rpc(
            handler=servicer.ListTaskPushNotificationConfig,
            request_deserializer=a2a__pb2.ListTaskPushNotificationConfigRequest.FromString,
            response_serializer=a2a__pb2.ListTaskPushNotificationConfigResponse.SerializeToString,
            request_streaming=False,
            response_streaming=False,
        ),
        "GetAgentCard": srpc.Rpc(
            handler=servicer.GetAgentCard,
            request_deserializer=a2a__pb2.GetAgentCardRequest.FromString,
            response_serializer=a2a__pb2.AgentCard.SerializeToString,
            request_streaming=False,
            response_streaming=False,
        ),
        "DeleteTaskPushNotificationConfig": srpc.Rpc(
            handler=servicer.DeleteTaskPushNotificationConfig,
            request_deserializer=a2a__pb2.DeleteTaskPushNotificationConfigRequest.FromString,
            response_serializer=google_dot_protobuf_dot_empty__pb2.Empty.SerializeToString,
            request_streaming=False,
            response_streaming=False,
        ),
    }
    server.add_registered_method_handlers("a2a.v1.A2AService", rpc_method_handlers)
