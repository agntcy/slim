In this example we start using the code in the slim/data-plane/python-bindings/examples/pubsub.py file on tag slim-bindings-v0.3.6 and we explain the modification required to move to slim-bindings-v0.4.0. 

```python
# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import argparse
import asyncio
import datetime

from common import split_id

import slim_bindings


async def run_client(local_id, remote_id, address, enable_opentelemetry: bool):
    # init tracing
    slim_bindings.init_tracing(
        {
            "log_level": "info",
            "opentelemetry": {
                "enabled": enable_opentelemetry,
                "grpc": {
                    "endpoint": "http://localhost:4317",
                },
            },
        }
    )

    # Split the local IDs into their respective components
    local_organization, local_namespace, local_agent = split_id(local_id)

    # Split the remote IDs into their respective components
    remote_organization, remote_namespace, broadcast_topic = split_id(remote_id)

    name = f"{local_agent}"

    print(f"Creating participant {name}...")

    participant = await slim_bindings.Slim.new(
        local_organization, local_namespace, local_agent
    )
```

The ```slim_bindings.Slim.new``` is now replaced by 
```python
slim_app = await Slim.new(local_name, provider, verifier)
```
you can see this in the [tutorial](https://docs.agntcy.org/messaging/slim-group-tutorial/#moderatorpy)
The provider and verifier are need to prove the identity of the application. There are to ways to setup them:
1. use a shared screet like presented in the [tutorail](https://docs.agntcy.org/messaging/slim-group-tutorial/#identity) and this si not racomanded for production ode
2. use a JWT token. In SLIM we use SPIRE to generate them and you can check you configure it in this other [tutorial](https://docs.agntcy.org/messaging/slim-group/#using-spire-with-slim)

```python
    # Connect to slim server
    _ = await participant.connect({"endpoint": address, "tls": {"insecure": True}})

    # set route for the chat, so that messages can be sent to the other participants
    await participant.set_route(remote_organization, remote_namespace, broadcast_topic)

    # Subscribe to the producer topic
    await participant.subscribe(remote_organization, remote_namespace, broadcast_topic)

    print(f"{name} -> Creating new pubsub sessions...")
    # create pubsubb session. A pubsub session is a just a bidirectional
    # streaming session, where participants are both sender and receivers
    session_info = await participant.create_session(
        slim_bindings.PySessionConfiguration.Streaming(
            slim_bindings.PySessionDirection.BIDIRECTIONAL,
            topic=slim_bindings.PyAgentType(
                remote_organization, remote_namespace, broadcast_topic
            ),
            max_retries=5,
            timeout=datetime.timedelta(seconds=5),
        )
    )
```
The connection command is the same as before, while for the routes and subscription you don't need to set them anymore as SLIM is taking care of that

In v0.3.6 each application has to setup a session as you can see in the previuos piece of code. Now this step is required only on the moderator applications. The parameters to set are the same, plus to new booleans: ```moderator = true``` and ```mls-enabled``` that needs to enabled to encrypts the packets  on the channel. [Here](https://docs.agntcy.org/messaging/slim-group-tutorial/#moderatorpy) you can find the full setup.

For all the other appplications that are not the moderator you need to listen for incoming messages. The reception works in a similar way to version 0.3.6. The old code was
```
    # define the background task
    async def background_task():
        msg = f"Hello from {local_agent}"

        async with participant:
            while True:
                try:
                    # receive message from session
                    recv_session, msg_rcv = await participant.receive(
                        session=session_info.id
                    )
```
Now the reception loop is presented [here](https://docs.agntcy.org/messaging/slim-group/#step-3-listen-for-invitations)

Another piece to add in the moderator app is the invite of all the participants as shown [here](https://docs.agntcy.org/messaging/slim-group/#step-2-invite-participants-to-the-channel). Without this the participants will not be able to get the packets shared on the channel. This is probably the main difference between the two version for the application point of view.

All the other interfaces did not change between the two versiosn. 
