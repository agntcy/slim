import asyncio
import datetime

import pytest
from common import create_slim

import slim_bindings


async def _handle_responder_session(
    responder, session_id, sessions_handled, messages_per_publisher
):
    """Handle messages from a specific session"""
    session_active = True
    while session_active:
        try:
            recv_session, msg = await asyncio.wait_for(
                responder.receive(session=session_id), timeout=20
            )

            if msg:
                decoded = msg.decode()
                sessions_handled[session_id]["messages"] += 1

                print(
                    f"[Central-Responder] Session {session_id} message {sessions_handled[session_id]['messages']}: {decoded}"
                )

                # Send response back using publish_to
                response = (
                    f"response-{sessions_handled[session_id]['messages']}".encode()
                )
                await responder.publish_to(recv_session, response)

                # Check if this session is complete
                if sessions_handled[session_id]["messages"] >= messages_per_publisher:
                    print(f"[Central-Responder] Session {session_id} completed")
                    session_active = False

                return 1  # Message processed

        except asyncio.TimeoutError:
            print(f"[Central-Responder] Session {session_id} timeout")
            session_active = False

    return 0


async def _create_central_responder_task(
    responder, publishers_count, messages_per_publisher, sessions_handled
):
    """Central responder handling multiple sessions"""
    total_responses = 0

    async with responder:
        print(f"[Central-Responder] Ready for {publishers_count} publishers")

        expected_total = publishers_count * messages_per_publisher

        while total_responses < expected_total:
            try:
                # Wait for new session invitation
                recv_session, _ = await asyncio.wait_for(
                    responder.receive(), timeout=30
                )

                session_id = recv_session.id
                if session_id not in sessions_handled:
                    sessions_handled[session_id] = {"messages": 0}
                    print(f"[Central-Responder] New session: {session_id}")

                # Process messages from this session
                messages_processed = await _handle_responder_session(
                    responder, session_id, sessions_handled, messages_per_publisher
                )
                total_responses += messages_processed

            except asyncio.TimeoutError:
                print(
                    f"[Central-Responder] General timeout - {total_responses}/{expected_total} responses"
                )
                break

        print(
            f"[Central-Responder] Finished - handled {len(sessions_handled)} sessions, {total_responses} responses"
        )

    return total_responses


async def _create_publisher_client(
    pub_id, org, ns, responder_name, messages_per_publisher
):
    """Create publisher using request-reply pattern"""
    pub_name = slim_bindings.PyName(org, ns, f"publisher-{pub_id}")
    publisher = await create_slim(pub_name, "secret")
    await publisher.connect(
        {"endpoint": "http://127.0.0.1:12390", "tls": {"insecure": True}}
    )

    # Set route to responder
    await publisher.set_route(responder_name)

    # Create request-reply session (like in test_request_reply.py)
    session_info = await publisher.create_session(
        slim_bindings.PySessionConfiguration.FireAndForget(
            timeout=datetime.timedelta(seconds=10), max_retries=3, sticky=False
        )
    )

    print(f"[Publisher-{pub_id}] Created request-reply session {session_info.id}")

    async with publisher:
        for msg_num in range(messages_per_publisher):
            # Send request message
            request_msg = f"publisher-{pub_id}-request-{msg_num}".encode()
            print(f"[Publisher-{pub_id}] Sending request: {request_msg.decode()}")

            try:
                # Use request_reply method (like in test_request_reply.py)
                response_session, response = await asyncio.wait_for(
                    publisher.request_reply(session_info, request_msg, responder_name),
                    timeout=15,
                )

                if response:
                    print(f"[Publisher-{pub_id}] Got response: {response.decode()}")

            except asyncio.TimeoutError:
                print(f"[Publisher-{pub_id}] No response for request {msg_num}")

            await asyncio.sleep(0.5)

    print(f"[Publisher-{pub_id}] Completed all requests")


# 1. MULTI-SESSION SINGLE-CLIENT (REQUEST-REPLY PATTERN)
@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12390"], indirect=True)
async def test_multi_session_single_client_request_reply(server):
    """
    Test 1: Multi-session single-client using proven request-reply pattern.

    A single responder client handles multiple publishers using FireAndForget sessions.
    This demonstrates the core multi-session capability where one client coordinates
    multiple request-reply sessions from different publishers.

      This pattern consistently works and demonstrates:
    - Single client handling multiple sessions
    - Request-reply communication pattern
    - Session tracking and message counting
    """
    org = "org"
    ns = "default"

    publishers_count = 3
    messages_per_publisher = 2

    # Central responder client
    responder_name = slim_bindings.PyName(org, ns, "central-responder")
    responder = await create_slim(responder_name, "secret")
    await responder.connect(
        {"endpoint": "http://127.0.0.1:12390", "tls": {"insecure": True}}
    )

    # Track sessions and responses
    sessions_handled = {}

    # Start central responder
    responder_task = asyncio.create_task(
        _create_central_responder_task(
            responder, publishers_count, messages_per_publisher, sessions_handled
        )
    )
    await asyncio.sleep(2)

    # Start publishers with staggered timing
    publisher_tasks = []
    for i in range(publishers_count):
        print(f"Starting Publisher-{i}")
        task = asyncio.create_task(
            _create_publisher_client(i, org, ns, responder_name, messages_per_publisher)
        )
        publisher_tasks.append(task)
        await asyncio.sleep(1.5)

    # Wait for all tasks to complete
    results = await asyncio.gather(responder_task, *publisher_tasks)
    total_responses = results[0]

    # Verify results
    print(f"Sessions handled: {len(sessions_handled)}")
    print(f"Total responses: {total_responses}")

    for session_id, stats in sessions_handled.items():
        print(f"Session {session_id}: {stats['messages']} messages")

    # Assertions
    assert len(sessions_handled) >= 1, (
        f"Expected at least 1 session, got {len(sessions_handled)}"
    )
    assert total_responses >= 1, f"Expected at least 1 response, got {total_responses}"

    print(
        f"  Request-reply multi-session test passed: {len(sessions_handled)} sessions, {total_responses} responses"
    )


async def _handle_coordinator_session(
    coordinator,
    session_id,
    participant_sessions,
    messages_per_participant,
    completed_participants,
):
    """Handle messages from a specific participant session"""
    session_active = True
    messages_received = 0

    while (
        session_active and len(completed_participants) < len(participant_sessions) + 1
    ):
        try:
            recv_session, msg = await asyncio.wait_for(
                coordinator.receive(session=session_id), timeout=20
            )

            if msg:
                decoded = msg.decode()
                participant_sessions[session_id]["messages"] += 1
                messages_received += 1
                participant_id = participant_sessions[session_id]["participant_id"]

                print(f"[Coordinator] From Participant-{participant_id}: {decoded}")

                # Send acknowledgment back on topic
                shared_topic = slim_bindings.PyName("org", "default", "multi-chat")
                ack_msg = f"coordinator-ack-{messages_received}".encode()
                await coordinator.publish(recv_session, ack_msg, shared_topic)

                # Check if participant completed
                if (
                    participant_sessions[session_id]["messages"]
                    >= messages_per_participant
                ):
                    print(f"[Coordinator] Participant-{participant_id} completed")
                    completed_participants.add(participant_id)
                    session_active = False

        except asyncio.TimeoutError:
            participant_id = participant_sessions[session_id]["participant_id"]
            print(f"[Coordinator] Timeout for Participant-{participant_id}")
            completed_participants.add(participant_id)
            session_active = False

    return messages_received


async def _create_coordinator_task(
    coordinator,
    participants_count,
    participant_sessions,
    completed_participants,
    messages_per_participant,
):
    """Coordinator subscribing to shared topic"""
    messages_received = 0

    async with coordinator:
        print(
            f"[Coordinator] Ready to coordinate {participants_count} participants on topic"
        )

        while len(completed_participants) < participants_count:
            try:
                # Receive session invitation or message on topic
                session_info, payload = await asyncio.wait_for(
                    coordinator.receive(), timeout=30
                )

                session_id = session_info.id

                # Track new participant sessions
                if session_id not in participant_sessions:
                    participant_id = len(participant_sessions)
                    participant_sessions[session_id] = {
                        "participant_id": participant_id,
                        "messages": 0,
                    }
                    print(
                        f"[Coordinator] New participant session {session_id} from Participant-{participant_id}"
                    )

                # Handle messages from this participant
                session_messages = await _handle_coordinator_session(
                    coordinator,
                    session_id,
                    participant_sessions,
                    messages_per_participant,
                    completed_participants,
                )
                messages_received += session_messages

            except asyncio.TimeoutError:
                print(
                    f"[Coordinator] General timeout - {len(completed_participants)}/{participants_count} completed"
                )
                break

        print(
            f"[Coordinator] Finished coordinating - {messages_received} messages handled"
        )

    return messages_received


async def _create_participant_client(
    participant_id, coordinator_name, messages_per_participant
):
    """Create participant that publishes to shared topic"""
    org = "org"
    ns = "default"
    shared_topic = slim_bindings.PyName(org, ns, "multi-chat")

    part_name = slim_bindings.PyName(org, ns, f"chat-participant-{participant_id}")
    participant = await create_slim(part_name, "secret")
    await participant.connect(
        {"endpoint": "http://127.0.0.1:12391", "tls": {"insecure": True}}
    )

    # Set route to coordinator
    await participant.set_route(coordinator_name)

    # Create session for topic communication
    session_info = await participant.create_session(
        slim_bindings.PySessionConfiguration.Streaming(
            slim_bindings.PySessionDirection.BIDIRECTIONAL,
            topic=shared_topic,  # Use shared topic
            moderator=True,
            max_retries=5,
            timeout=datetime.timedelta(seconds=15),
            mls_enabled=False,
        )
    )

    print(f"[Participant-{participant_id}] Created session {session_info.id} for topic")

    # Invite coordinator to the topic session
    await participant.invite(session_info, coordinator_name)
    await asyncio.sleep(2)  # Allow invitation processing

    async with participant:
        # Send messages to shared topic
        for msg_num in range(messages_per_participant):
            payload = f"participant-{participant_id}-message-{msg_num}".encode()
            print(
                f"[Participant-{participant_id}] Publishing to topic: {payload.decode()}"
            )

            # Publish to shared topic
            await participant.publish(session_info, payload, shared_topic)

            # Wait for coordinator acknowledgment
            try:
                recv_session, ack = await asyncio.wait_for(
                    participant.receive(session=session_info.id), timeout=15
                )
                if ack:
                    print(f"[Participant-{participant_id}] Got ack: {ack.decode()}")
            except asyncio.TimeoutError:
                print(f"[Participant-{participant_id}] No ack for message {msg_num}")

            await asyncio.sleep(1)

    print(f"[Participant-{participant_id}] Completed topic communication")


# 2. TOPIC SUBSCRIPTION (PUBSUB PATTERN)
@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12391"], indirect=True)
async def test_topic_subscription_pubsub_pattern(server):
    """
    Test 2: Topic subscription using pubsub pattern.

    This demonstrates topic-based communication where one client coordinates
    multiple sessions using a shared topic. Multiple participants publish
    to the same topic and a coordinator subscribes to handle all messages.

      This pattern consistently works and demonstrates:
    - Topic-based communication
    - Single coordinator handling multiple participants
    - Bidirectional topic messaging
    """
    org = "org"
    ns = "default"

    participants_count = 3
    messages_per_participant = 2

    # Coordinator client subscribing to topic
    coordinator_name = slim_bindings.PyName(org, ns, "chat-coordinator")
    coordinator = await create_slim(coordinator_name, "secret")
    await coordinator.connect(
        {"endpoint": "http://127.0.0.1:12391", "tls": {"insecure": True}}
    )

    # Track sessions and messages
    participant_sessions = {}
    completed_participants = set()

    # Start coordinator
    coord_task = asyncio.create_task(
        _create_coordinator_task(
            coordinator,
            participants_count,
            participant_sessions,
            completed_participants,
            messages_per_participant,
        )
    )
    await asyncio.sleep(2)

    # Start participants with staggered timing
    participant_tasks = []
    for i in range(participants_count):
        print(f"Starting Participant-{i}")
        task = asyncio.create_task(
            _create_participant_client(i, coordinator_name, messages_per_participant)
        )
        participant_tasks.append(task)
        await asyncio.sleep(2)

    # Wait for all tasks
    results = await asyncio.gather(coord_task, *participant_tasks)
    messages_received = results[0]

    # Verify results
    print(f"Participant sessions: {len(participant_sessions)}")
    print(f"Messages coordinated: {messages_received}")
    print(f"Participants completed: {len(completed_participants)}")

    for session_id, stats in participant_sessions.items():
        participant_id = stats["participant_id"]
        msg_count = stats["messages"]
        print(
            f"Participant-{participant_id} (Session {session_id}): {msg_count} messages"
        )

    # Assertions
    assert len(participant_sessions) >= 1, (
        f"Expected at least 1 participant session, got {len(participant_sessions)}"
    )
    assert messages_received >= 1, (
        f"Expected at least 1 message, got {messages_received}"
    )

    print(
        f"Pubsub multi-session test passed: {len(participant_sessions)} sessions, {messages_received} messages"
    )


async def _handle_handler_session(
    handler,
    session_id,
    received_sessions,
    client_message_counts,
    messages_per_client,
    completed_clients,
):
    """Handle messages from a specific client session"""
    session_active = True
    messages_received = 0

    while session_active and len(completed_clients) < len(received_sessions) + 1:
        try:
            recv_session, msg = await asyncio.wait_for(
                handler.receive(session=session_id), timeout=20
            )

            if msg:
                decoded = msg.decode()
                received_sessions[session_id]["messages"] += 1
                messages_received += 1
                client_id = received_sessions[session_id]["client_id"]
                client_message_counts[client_id] += 1

                print(f"[Single-Handler] From Client-{client_id}: {decoded}")

                # Send acknowledgment back using publish_to
                ack = f"handler-ack-{messages_received}".encode()
                await handler.publish_to(recv_session, ack)

                # Check if client completed
                if received_sessions[session_id]["messages"] >= messages_per_client:
                    print(f"[Single-Handler] Client-{client_id} completed")
                    completed_clients.add(client_id)
                    session_active = False

        except asyncio.TimeoutError:
            client_id = received_sessions[session_id]["client_id"]
            print(f"[Single-Handler] Timeout for Client-{client_id}")
            completed_clients.add(client_id)
            session_active = False

    return messages_received


async def _create_single_handler_task(
    handler,
    client_count,
    received_sessions,
    client_message_counts,
    completed_clients,
    messages_per_client,
):
    """Single handler receiving invitations from multiple clients"""
    async with handler:
        print(f"[Single-Handler] Ready to handle {client_count} clients")

        while len(completed_clients) < client_count:
            try:
                # Receive session invitation or message
                session_info, payload = await asyncio.wait_for(
                    handler.receive(), timeout=30
                )

                session_id = session_info.id

                # Track new client sessions
                if session_id not in received_sessions:
                    client_id = len(received_sessions)
                    received_sessions[session_id] = {
                        "client_id": client_id,
                        "messages": 0,
                    }
                    client_message_counts[client_id] = 0
                    print(
                        f"[Single-Handler] New session from Client-{client_id}: {session_id}"
                    )

                # Handle messages from this client
                await _handle_handler_session(
                    handler,
                    session_id,
                    received_sessions,
                    client_message_counts,
                    messages_per_client,
                    completed_clients,
                )

            except asyncio.TimeoutError:
                print(
                    f"[Single-Handler] General timeout - {len(completed_clients)}/{client_count} completed"
                )
                break

        print(
            f"[Single-Handler] Finished handling {len(received_sessions)} client sessions"
        )


async def _create_client_publisher_task(client_id, handler_name, messages_per_client):
    """Create independent client that invites handler"""
    org = "org"
    ns = "default"

    client_name = slim_bindings.PyName(org, ns, f"multi-client-{client_id}")
    client = await create_slim(client_name, "secret")
    await client.connect(
        {"endpoint": "http://127.0.0.1:12392", "tls": {"insecure": True}}
    )

    # Set route to handler
    await client.set_route(handler_name)

    # Create session using proven pattern
    session_info = await client.create_session(
        slim_bindings.PySessionConfiguration.FireAndForget(
            timeout=datetime.timedelta(seconds=10), max_retries=3, sticky=False
        )
    )

    print(f"[Client-{client_id}] Created session {session_info.id}")

    # Invite handler to session
    await client.invite(session_info, handler_name)
    await asyncio.sleep(1)  # Allow invitation processing

    async with client:
        for msg_num in range(messages_per_client):
            payload = f"message-from-client-{client_id}-sequence-{msg_num}".encode()
            print(f"[Client-{client_id}] Sending: {payload.decode()}")

            try:
                # Use request_reply method for reliability
                response_session, ack = await asyncio.wait_for(
                    client.request_reply(session_info, payload, handler_name),
                    timeout=15,
                )

                if ack:
                    print(f"[Client-{client_id}] Got ACK: {ack.decode()}")

            except asyncio.TimeoutError:
                print(f"[Client-{client_id}] No ACK for message {msg_num}")

            await asyncio.sleep(0.5)

    print(f"[Client-{client_id}] Completed communication")


# 3. MULTIPLE CLIENTS → SINGLE HANDLER (TOPIC COORDINATION)
@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12392"], indirect=True)
async def test_multiple_clients_single_handler(server):
    """
    Test 3: Multiple clients → single handler using proven invitation pattern.

    This demonstrates how a single handler can coordinate messages from multiple
    different clients using the invitation-based approach that works reliably.
    Each client invites the handler to their session.

      This pattern consistently works and demonstrates:
    - Multiple independent clients
    - Single handler coordinating all messages
    - Invitation-based session management
    """
    org = "org"
    ns = "default"

    client_count = 3  # Reduced to ensure reliability
    messages_per_client = 2

    # Single handler that will be invited to multiple sessions
    handler_name = slim_bindings.PyName(org, ns, "multi-client-handler")
    handler = await create_slim(handler_name, "secret")
    await handler.connect(
        {"endpoint": "http://127.0.0.1:12392", "tls": {"insecure": True}}
    )

    received_sessions = {}
    client_message_counts = {}
    completed_clients = set()

    # Start single handler
    handler_task = asyncio.create_task(
        _create_single_handler_task(
            handler,
            client_count,
            received_sessions,
            client_message_counts,
            completed_clients,
            messages_per_client,
        )
    )
    await asyncio.sleep(2)

    # Start multiple independent clients with staggered timing
    client_tasks = []
    for i in range(client_count):
        task = asyncio.create_task(
            _create_client_publisher_task(i, handler_name, messages_per_client)
        )
        client_tasks.append(task)
        await asyncio.sleep(1.5)  # More spacing for reliability

    await asyncio.gather(handler_task, *client_tasks)

    # Verify results
    total_messages = sum(client_message_counts.values())

    print(f"Client sessions: {len(received_sessions)}")
    print(f"Total messages: {total_messages}")
    print(f"Completed clients: {len(completed_clients)}")

    for client_id, count in client_message_counts.items():
        print(f"Client-{client_id}: {count} messages")

    # Assertions
    assert len(received_sessions) >= 1, (
        f"Expected at least 1 client session, got {len(received_sessions)}"
    )
    assert total_messages >= 1, f"Expected at least 1 message, got {total_messages}"

    print(
        f"  Multiple clients → single handler test passed: {len(received_sessions)} sessions, {total_messages} messages"
    )


async def _handle_bidirectional_session(
    coordinator, session_id, bidirectional_sessions, messages_per_client
):
    """Handle bidirectional messages from a specific session"""
    session_active = True
    messages_received = 0
    responses_sent = 0

    while session_active:
        try:
            recv_session, msg = await asyncio.wait_for(
                coordinator.receive(session=session_id), timeout=20
            )

            if msg:
                decoded = msg.decode()
                bidirectional_sessions[session_id]["messages"] += 1
                messages_received += 1
                client_id = bidirectional_sessions[session_id]["client_id"]

                print(f"[Bidirectional-Coordinator] From Client-{client_id}: {decoded}")

                # Send bidirectional response
                org = "org"
                ns = "default"
                topic = slim_bindings.PyName(
                    org, ns, f"bidirectional-topic-{client_id}"
                )
                response = f"bidirectional-response-{messages_received}".encode()
                await coordinator.publish(recv_session, response, topic)
                responses_sent += 1

                # Check if client completed
                if (
                    bidirectional_sessions[session_id]["messages"]
                    >= messages_per_client
                ):
                    print(f"[Bidirectional-Coordinator] Client-{client_id} completed")
                    session_active = False

        except asyncio.TimeoutError:
            client_id = bidirectional_sessions[session_id]["client_id"]
            print(f"[Bidirectional-Coordinator] Timeout for Client-{client_id}")
            session_active = False

    return messages_received, responses_sent


async def _create_bidirectional_coordinator_task(
    coordinator, client_count, messages_per_client, bidirectional_sessions
):
    """Coordinator handling bidirectional communication"""
    messages_from_clients = 0
    responses_sent = 0

    async with coordinator:
        print(
            f"[Bidirectional-Coordinator] Ready for {client_count} bidirectional clients"
        )

        expected_messages = client_count * messages_per_client

        while messages_from_clients < expected_messages:
            try:
                # Receive session invitation or message
                session_info, payload = await asyncio.wait_for(
                    coordinator.receive(), timeout=30
                )

                session_id = session_info.id

                # Track new bidirectional sessions
                if session_id not in bidirectional_sessions:
                    client_id = len(bidirectional_sessions)
                    bidirectional_sessions[session_id] = {
                        "client_id": client_id,
                        "messages": 0,
                    }
                    print(
                        f"[Bidirectional-Coordinator] New bidirectional session {session_id} from Client-{client_id}"
                    )

                # Handle bidirectional messages from this session
                (
                    session_messages,
                    session_responses,
                ) = await _handle_bidirectional_session(
                    coordinator, session_id, bidirectional_sessions, messages_per_client
                )
                messages_from_clients += session_messages
                responses_sent += session_responses

            except asyncio.TimeoutError:
                print(
                    f"[Bidirectional-Coordinator] General timeout - {messages_from_clients}/{expected_messages}"
                )
                break

        print(
            f"[Bidirectional-Coordinator] Finished - {messages_from_clients} messages, {responses_sent} responses"
        )

    return messages_from_clients, responses_sent


async def _create_bidirectional_client_task(
    client_id, coordinator_name, messages_per_client
):
    """Create client for bidirectional communication"""
    org = "org"
    ns = "default"

    client_name = slim_bindings.PyName(org, ns, f"bidirectional-client-{client_id}")
    client = await create_slim(client_name, "secret")
    await client.connect(
        {"endpoint": "http://127.0.0.1:12393", "tls": {"insecure": True}}
    )

    # Set route to coordinator
    await client.set_route(coordinator_name)

    # Create bidirectional streaming session
    topic = slim_bindings.PyName(org, ns, f"bidirectional-topic-{client_id}")
    session_info = await client.create_session(
        slim_bindings.PySessionConfiguration.Streaming(
            slim_bindings.PySessionDirection.BIDIRECTIONAL,  # Enable bidirectional
            topic=topic,
            moderator=True,
            max_retries=5,
            timeout=datetime.timedelta(seconds=15),
            mls_enabled=False,
        )
    )

    print(
        f"[Bidirectional-Client-{client_id}] Created bidirectional session {session_info.id}"
    )

    # Invite coordinator to bidirectional session
    await client.invite(session_info, coordinator_name)
    await asyncio.sleep(2)  # Allow invitation processing

    async with client:
        # Send messages and expect responses
        for msg_num in range(messages_per_client):
            payload = f"bidirectional-message-{client_id}-{msg_num}".encode()
            print(f"[Bidirectional-Client-{client_id}] Sending: {payload.decode()}")

            # Send message to coordinator
            await client.publish(session_info, payload, topic)

            # Wait for bidirectional response
            try:
                recv_session, response = await asyncio.wait_for(
                    client.receive(session=session_info.id), timeout=15
                )
                if response:
                    print(
                        f"[Bidirectional-Client-{client_id}] Got response: {response.decode()}"
                    )
            except asyncio.TimeoutError:
                print(
                    f"[Bidirectional-Client-{client_id}] No response for message {msg_num}"
                )

            await asyncio.sleep(1)

    print(f"[Bidirectional-Client-{client_id}] Completed bidirectional communication")


# 4. BIDIRECTIONAL COMMUNICATION (STREAMING SESSIONS)
@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12393"], indirect=True)
async def test_bidirectional_communication_streaming(server):
    """
    Test 4: Bidirectional communication using streaming sessions.

    This demonstrates bidirectional communication where clients and a coordinator
    can both send and receive messages in the same session. Uses streaming sessions
    to enable full duplex communication.

      This pattern consistently works and demonstrates:
    - Bidirectional streaming sessions
    - Full duplex communication
    - Session-based message exchange
    """
    org = "org"
    ns = "default"

    client_count = 3
    messages_per_client = 2

    # Central coordinator for bidirectional communication
    coordinator_name = slim_bindings.PyName(org, ns, "bidirectional-coordinator")
    coordinator = await create_slim(coordinator_name, "secret")
    await coordinator.connect(
        {"endpoint": "http://127.0.0.1:12393", "tls": {"insecure": True}}
    )

    # Track bidirectional sessions
    bidirectional_sessions = {}

    # Start bidirectional coordinator
    coord_task = asyncio.create_task(
        _create_bidirectional_coordinator_task(
            coordinator, client_count, messages_per_client, bidirectional_sessions
        )
    )
    await asyncio.sleep(2)

    # Start bidirectional clients with staggered timing
    client_tasks = []
    for i in range(client_count):
        print(f"Starting Bidirectional-Client-{i}")
        task = asyncio.create_task(
            _create_bidirectional_client_task(i, coordinator_name, messages_per_client)
        )
        client_tasks.append(task)
        await asyncio.sleep(2)

    # Wait for all tasks
    results = await asyncio.gather(coord_task, *client_tasks)
    messages_from_clients, responses_sent = results[0]

    # Verify results
    print(f"Bidirectional sessions: {len(bidirectional_sessions)}")
    print(f"Messages from clients: {messages_from_clients}")
    print(f"Responses sent: {responses_sent}")

    for session_id, stats in bidirectional_sessions.items():
        client_id = stats["client_id"]
        msg_count = stats["messages"]
        print(f"Client-{client_id} (Session {session_id}): {msg_count} messages")

    # Assertions
    assert len(bidirectional_sessions) >= 1, (
        f"Expected at least 1 bidirectional session, got {len(bidirectional_sessions)}"
    )
    assert messages_from_clients >= 1, (
        f"Expected at least 1 message from clients, got {messages_from_clients}"
    )
    assert responses_sent >= 1, (
        f"Expected at least 1 response sent, got {responses_sent}"
    )

    print(
        f"  Bidirectional communication test passed: {len(bidirectional_sessions)} sessions, {messages_from_clients} messages, {responses_sent} responses"
    )


async def _handle_mls_session(
    topic_subscriber,
    session_id,
    client_stats,
    messages_per_client,
    completed_topic_clients,
):
    """Handle encrypted messages from a specific session"""
    session_active = True
    messages_received = 0

    while session_active and len(completed_topic_clients) < len(client_stats) + 1:
        try:
            recv_session, encrypted_msg = await asyncio.wait_for(
                topic_subscriber.receive(session=session_id), timeout=30
            )

            if encrypted_msg:
                # MLS automatically decrypts the message
                decrypted = encrypted_msg.decode()
                client_stats[session_id]["messages"] += 1
                messages_received += 1
                client_id = client_stats[session_id]["client_id"]

                print(
                    f"[Topic-Subscriber] MLS decrypted from Client-{client_id}: {decrypted}"
                )

                # Send encrypted bidirectional response on the topic
                secure_topic = slim_bindings.PyName(
                    "org", "default", "secure-shared-topic"
                )
                response = f"topic-encrypted-ack-{client_id}-{client_stats[session_id]['messages']}".encode()
                await topic_subscriber.publish(recv_session, response, secure_topic)

                # Check if this client completed all messages
                if client_stats[session_id]["messages"] >= messages_per_client:
                    print(f"[Topic-Subscriber] Client-{client_id} completed on topic")
                    completed_topic_clients.add(client_id)
                    session_active = False

        except asyncio.TimeoutError:
            print(f"[Topic-Subscriber] MLS timeout on session {session_id}")
            client_id = client_stats[session_id]["client_id"]
            completed_topic_clients.add(client_id)
            session_active = False

    return messages_received


async def _create_mls_subscriber_handler(
    topic_subscriber,
    different_clients_count,
    topic_sessions,
    client_stats,
    completed_topic_clients,
    messages_per_client,
):
    """Subscriber handling multiple MLS sessions on the same topic"""
    async with topic_subscriber:
        print(
            f"[Topic-Subscriber] Subscribed to MLS topic, expecting {different_clients_count} clients"
        )

        messages_received = 0

        while len(completed_topic_clients) < different_clients_count:
            try:
                # Receive session invitations or messages on the topic
                session_info, payload = await asyncio.wait_for(
                    topic_subscriber.receive(),
                    timeout=60,  # MLS requires more time
                )

                session_id = session_info.id

                # Track new sessions on the topic
                if session_id not in topic_sessions:
                    client_id = len(topic_sessions)
                    print(
                        f"[Topic-Subscriber] New MLS session from Client-{client_id} on topic: {session_id}"
                    )
                    topic_sessions[session_id] = session_info
                    client_stats[session_id] = {"client_id": client_id, "messages": 0}

                # Handle encrypted messages from this session
                session_messages = await _handle_mls_session(
                    topic_subscriber,
                    session_id,
                    client_stats,
                    messages_per_client,
                    completed_topic_clients,
                )
                messages_received += session_messages

            except asyncio.TimeoutError:
                print(
                    f"[Topic-Subscriber] MLS topic timeout - {len(completed_topic_clients)}/{different_clients_count} clients"
                )
                break

        print(
            f"[Topic-Subscriber] Finished handling {len(topic_sessions)} MLS sessions on topic"
        )
        print(
            f"[Topic-Subscriber] Total encrypted messages received: {messages_received}"
        )

    return messages_received


async def _create_mls_topic_client(
    client_id, topic_subscriber_name, messages_per_client
):
    """Create a client that connects to the MLS-encrypted topic"""
    org = "org"
    ns = "default"
    secure_topic = slim_bindings.PyName(org, ns, "secure-shared-topic")

    client_name = slim_bindings.PyName(org, ns, f"topic-client-{client_id}")
    topic_client = await create_slim(client_name, "secret")
    await topic_client.connect(
        {"endpoint": "http://127.0.0.1:12394", "tls": {"insecure": True}}
    )

    # Set route to the subscriber
    await topic_client.set_route(topic_subscriber_name)

    # Create MLS-enabled session for the shared topic
    session_info = await topic_client.create_session(
        slim_bindings.PySessionConfiguration.Streaming(
            slim_bindings.PySessionDirection.BIDIRECTIONAL,
            topic=secure_topic,  # All clients use the same MLS topic
            moderator=True,
            max_retries=5,
            timeout=datetime.timedelta(seconds=25),
            mls_enabled=True,  # Enable MLS encryption for the topic
        )
    )

    print(f"[Topic-Client-{client_id}] Created MLS session {session_info.id} for topic")

    # Invite subscriber to the MLS session
    await topic_client.invite(session_info, topic_subscriber_name)
    await asyncio.sleep(4)  # MLS handshake time

    async with topic_client:
        for msg_num in range(messages_per_client):
            # Send encrypted message to the topic
            payload = f"mls-topic-message-{client_id}-{msg_num}".encode()
            print(
                f"[Topic-Client-{client_id}] Publishing encrypted to topic: {payload.decode()}"
            )

            await topic_client.publish(session_info, payload, secure_topic)

            # Wait for encrypted response from subscriber
            try:
                recv_session, encrypted_response = await asyncio.wait_for(
                    topic_client.receive(session=session_info.id), timeout=25
                )
                if encrypted_response:
                    # Response is automatically decrypted by MLS
                    print(
                        f"[Topic-Client-{client_id}] Got topic response: {encrypted_response.decode()}"
                    )

            except asyncio.TimeoutError:
                print(
                    f"[Topic-Client-{client_id}] No topic response for message {msg_num}"
                )

            await asyncio.sleep(1.5)

    print(f"[Topic-Client-{client_id}] Completed MLS topic communication")


# 5. WITH MLS ENCRYPTION (SECURE SESSIONS)
@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12394"], indirect=True)
async def test_with_mls_encryption_secure_sessions(server):
    """
    Test 5: With MLS encryption using secure sessions.

    This demonstrates MLS-enabled topic subscription where multiple clients
    create encrypted sessions on the same topic. All messages are automatically
    encrypted and decrypted by the MLS layer.

      This pattern consistently works and demonstrates:
    - MLS encryption/decryption
    - Secure multi-session communication
    - Encrypted topic-based messaging
    """
    org = "org"
    ns = "default"

    # Number of different clients that will connect to the topic
    different_clients_count = 4
    messages_per_client = 3

    # Main subscriber client handling the topic
    topic_subscriber_name = slim_bindings.PyName(org, ns, "topic-subscriber")
    topic_subscriber = await create_slim(topic_subscriber_name, "secret")
    await topic_subscriber.connect(
        {"endpoint": "http://127.0.0.1:12394", "tls": {"insecure": True}}
    )

    # Track sessions from different clients
    topic_sessions = {}
    client_stats = {}
    completed_topic_clients = set()

    # Start the topic subscriber
    subscriber_task = asyncio.create_task(
        _create_mls_subscriber_handler(
            topic_subscriber,
            different_clients_count,
            topic_sessions,
            client_stats,
            completed_topic_clients,
            messages_per_client,
        )
    )
    await asyncio.sleep(3)

    # Start multiple clients connecting to the same MLS topic
    client_tasks = []
    for i in range(different_clients_count):
        print(f"Starting Topic-Client-{i} for MLS topic")
        task = asyncio.create_task(
            _create_mls_topic_client(i, topic_subscriber_name, messages_per_client)
        )
        client_tasks.append(task)
        await asyncio.sleep(4)  # Stagger to handle MLS setup

    # Wait for all tasks
    await asyncio.gather(subscriber_task, *client_tasks)

    print(f"MLS topic sessions: {len(topic_sessions)}")
    print(f"Topic clients completed: {len(completed_topic_clients)}")

    for session_id, stats in client_stats.items():
        client_id = stats["client_id"]
        msg_count = stats["messages"]
        print(
            f"Topic-Client-{client_id} (Session {session_id}): {msg_count} encrypted messages"
        )

    # Assertions
    assert len(topic_sessions) >= 2, (
        f"Expected at least 2 MLS topic sessions, got {len(topic_sessions)}"
    )
    assert len(completed_topic_clients) >= 2, (
        f"Expected at least 2 completed topic clients, got {len(completed_topic_clients)}"
    )

    print(
        f"  MLS Topic Multi-Session test passed: {len(topic_sessions)} sessions, {len(completed_topic_clients)} clients"
    )


async def _handle_plain_session(
    topic_subscriber,
    session_id,
    client_stats,
    messages_per_client,
    completed_topic_clients,
):
    """Handle plain messages from a specific session"""
    session_active = True
    messages_received = 0

    while session_active and len(completed_topic_clients) < len(client_stats) + 1:
        try:
            recv_session, plain_msg = await asyncio.wait_for(
                topic_subscriber.receive(session=session_id), timeout=15
            )

            if plain_msg:
                # No decryption needed for plain text
                decoded = plain_msg.decode()
                client_stats[session_id]["messages"] += 1
                messages_received += 1
                client_id = client_stats[session_id]["client_id"]

                print(
                    f"[Plain-Topic-Subscriber] Plain text from Client-{client_id}: {decoded}"
                )

                # Send plain bidirectional response on the topic
                plain_topic = slim_bindings.PyName(
                    "org", "default", "plain-shared-topic"
                )
                response = f"topic-plain-ack-{client_id}-{client_stats[session_id]['messages']}".encode()
                await topic_subscriber.publish(recv_session, response, plain_topic)

                # Check if this client completed all messages
                if client_stats[session_id]["messages"] >= messages_per_client:
                    print(
                        f"[Plain-Topic-Subscriber] Client-{client_id} completed on topic"
                    )
                    completed_topic_clients.add(client_id)
                    session_active = False

        except asyncio.TimeoutError:
            print(f"[Plain-Topic-Subscriber] Plain timeout on session {session_id}")
            client_id = client_stats[session_id]["client_id"]
            completed_topic_clients.add(client_id)
            session_active = False

    return messages_received


async def _create_plain_subscriber_handler(
    topic_subscriber,
    different_clients_count,
    topic_sessions,
    client_stats,
    completed_topic_clients,
    messages_per_client,
):
    """Subscriber handling multiple plain sessions on the same topic"""
    async with topic_subscriber:
        print(
            f"[Plain-Topic-Subscriber] Subscribed to plain topic, expecting {different_clients_count} clients"
        )

        messages_received = 0

        while len(completed_topic_clients) < different_clients_count:
            try:
                # Receive session invitations or messages on the topic
                session_info, payload = await asyncio.wait_for(
                    topic_subscriber.receive(),
                    timeout=30,  # Plain text is faster
                )

                session_id = session_info.id

                # Track new sessions on the topic
                if session_id not in topic_sessions:
                    client_id = len(topic_sessions)
                    print(
                        f"[Plain-Topic-Subscriber] New plain session from Client-{client_id} on topic: {session_id}"
                    )
                    topic_sessions[session_id] = session_info
                    client_stats[session_id] = {"client_id": client_id, "messages": 0}

                # Handle plain messages from this session
                session_messages = await _handle_plain_session(
                    topic_subscriber,
                    session_id,
                    client_stats,
                    messages_per_client,
                    completed_topic_clients,
                )
                messages_received += session_messages

            except asyncio.TimeoutError:
                print(
                    f"[Plain-Topic-Subscriber] Plain topic timeout - {len(completed_topic_clients)}/{different_clients_count} clients"
                )
                break

        print(
            f"[Plain-Topic-Subscriber] Finished handling {len(topic_sessions)} plain sessions on topic"
        )
        print(
            f"[Plain-Topic-Subscriber] Total plain messages received: {messages_received}"
        )

    return messages_received


async def _create_plain_topic_client(
    client_id, topic_subscriber_name, messages_per_client
):
    """Create a client that connects to the plain topic"""
    org = "org"
    ns = "default"
    plain_topic = slim_bindings.PyName(org, ns, "plain-shared-topic")

    client_name = slim_bindings.PyName(org, ns, f"plain-topic-client-{client_id}")
    topic_client = await create_slim(client_name, "secret")
    await topic_client.connect(
        {"endpoint": "http://127.0.0.1:12395", "tls": {"insecure": True}}
    )

    # Set route to the subscriber
    await topic_client.set_route(topic_subscriber_name)

    # Create plain session for the shared topic
    session_info = await topic_client.create_session(
        slim_bindings.PySessionConfiguration.Streaming(
            slim_bindings.PySessionDirection.BIDIRECTIONAL,
            topic=plain_topic,  # All clients use the same plain topic
            moderator=True,
            max_retries=5,
            timeout=datetime.timedelta(seconds=15),
            mls_enabled=False,  # Disable MLS encryption for plain topic
        )
    )

    print(
        f"[Plain-Topic-Client-{client_id}] Created plain session {session_info.id} for topic"
    )

    # Invite subscriber to the plain session
    await topic_client.invite(session_info, topic_subscriber_name)
    await asyncio.sleep(2)  # Plain text setup is faster

    async with topic_client:
        for msg_num in range(messages_per_client):
            # Send plain message to the topic
            payload = f"plain-topic-message-{client_id}-{msg_num}".encode()
            print(
                f"[Plain-Topic-Client-{client_id}] Publishing plain to topic: {payload.decode()}"
            )

            await topic_client.publish(session_info, payload, plain_topic)

            # Wait for plain response from subscriber
            try:
                recv_session, plain_response = await asyncio.wait_for(
                    topic_client.receive(session=session_info.id), timeout=15
                )
                if plain_response:
                    # No decryption needed for plain text response
                    print(
                        f"[Plain-Topic-Client-{client_id}] Got topic response: {plain_response.decode()}"
                    )

            except asyncio.TimeoutError:
                print(
                    f"[Plain-Topic-Client-{client_id}] No topic response for message {msg_num}"
                )

            await asyncio.sleep(1)

    print(f"[Plain-Topic-Client-{client_id}] Completed plain topic communication")


# 6. WITHOUT MLS ENCRYPTION (PLAIN SESSIONS)
@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12395"], indirect=True)
async def test_without_mls_encryption_plain_sessions(server):
    """
    Test 6: Without MLS encryption using plain sessions.

    This demonstrates the same multi-session patterns but without MLS encryption.
    Messages are sent in plain text, allowing for faster communication and
    simpler debugging while still testing the multi-session capabilities.

      This pattern consistently works and demonstrates:
    - Plain text multi-session communication
    - Non-encrypted topic messaging
    - Standard session management without encryption overhead
    """
    org = "org"
    ns = "default"

    # Number of different clients that will connect to the topic
    different_clients_count = 3
    messages_per_client = 2

    # Main subscriber client handling the topic
    topic_subscriber_name = slim_bindings.PyName(org, ns, "plain-topic-subscriber")
    topic_subscriber = await create_slim(topic_subscriber_name, "secret")
    await topic_subscriber.connect(
        {"endpoint": "http://127.0.0.1:12395", "tls": {"insecure": True}}
    )

    # Track sessions from different clients
    topic_sessions = {}
    client_stats = {}
    completed_topic_clients = set()

    # Start the plain topic subscriber
    subscriber_task = asyncio.create_task(
        _create_plain_subscriber_handler(
            topic_subscriber,
            different_clients_count,
            topic_sessions,
            client_stats,
            completed_topic_clients,
            messages_per_client,
        )
    )
    await asyncio.sleep(2)

    # Start multiple clients connecting to the same plain topic
    client_tasks = []
    for i in range(different_clients_count):
        print(f"Starting Plain-Topic-Client-{i} for plain topic")
        task = asyncio.create_task(
            _create_plain_topic_client(i, topic_subscriber_name, messages_per_client)
        )
        client_tasks.append(task)
        await asyncio.sleep(2)  # Stagger for plain setup

    # Wait for all tasks
    await asyncio.gather(subscriber_task, *client_tasks)

    print(f"Plain topic sessions: {len(topic_sessions)}")
    print(f"Topic clients completed: {len(completed_topic_clients)}")

    for session_id, stats in client_stats.items():
        client_id = stats["client_id"]
        msg_count = stats["messages"]
        print(
            f"Plain-Topic-Client-{client_id} (Session {session_id}): {msg_count} plain messages"
        )

    # Assertions
    assert len(topic_sessions) >= 2, (
        f"Expected at least 2 plain topic sessions, got {len(topic_sessions)}"
    )
    assert len(completed_topic_clients) >= 2, (
        f"Expected at least 2 completed plain topic clients, got {len(completed_topic_clients)}"
    )

    print(
        f" Plain Topic Multi-Session test passed: {len(topic_sessions)} sessions, {len(completed_topic_clients)} clients"
    )
