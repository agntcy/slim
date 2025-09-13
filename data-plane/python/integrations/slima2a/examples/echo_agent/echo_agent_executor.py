import logging

from a2a.server.agent_execution import AgentExecutor, RequestContext
from a2a.server.events import EventQueue
from a2a.server.tasks.task_updater import TaskUpdater
from a2a.types import Message, Part, Role, TextPart

from examples.echo_agent.echo_agent import EchoAgent


class EchoAgentExecutor(AgentExecutor):
    def __init__(self) -> None:
        self.agent = EchoAgent()

    async def execute(
        self,
        context: RequestContext,
        event_queue: EventQueue,
    ) -> None:
        if (
            (not context.message)
            or (not context.message.task_id)
            or (not context.message.context_id)
        ):
            raise Exception("invalid message")

        logging.debug(f"received message: {context.message}")

        task_updater = TaskUpdater(
            event_queue=event_queue,
            task_id=context.message.task_id,
            context_id=context.message.context_id,
        )
        await task_updater.submit(message=context.message)

        if type(context.message.parts[0].root) is not TextPart:
            raise Exception("only text parts are supported")

        result = await self.agent.invoke(context.message.parts[0].root.text)

        response = Message(
            role=Role.agent,
            message_id=context.message.message_id,
            parts=[Part(root=TextPart(text=result))],
        )
        await task_updater.add_artifact(
            parts=response.parts,
            name="result",
        )
        await task_updater.complete(message=response)

    async def cancel(self, context: RequestContext, event_queue: EventQueue) -> None:
        raise NotImplementedError("cancel not supported")
