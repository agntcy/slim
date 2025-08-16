import asyncio
import datetime

import pytest

from common import create_slim
import slim_bindings

@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12390"], indirect=True)
async def test_multi_session_single_client(server):
    """
    Non-MLS variant: multiple publishers each create a bidirectional session and invite the single subscriber.
    """

    org = "org"
    ns = "default"
    topic_str = "multi-session"
    topic = slim_bindings.PyName(org, ns, topic_str)

    mls_enabled = False
    # Fresh state per MLS mode
    publishers_count = 2
    messages_per_publisher = 2
    sub_name = slim_bindings.PyName(org, ns, f"subscriber-mls-{mls_enabled}")
    subscriber = await create_slim(sub_name, "secret")
    _ = await subscriber.connect({"endpoint": "http://127.0.0.1:12390", "tls": {"insecure": True}})
    # No subscription needed; we'll be directly invited to each session

    session_message_counts = {}
    session_infos = {}
    ready_event = asyncio.Event()

    async def subscriber_task():  # noqa: C901
        ready_event.set()
        async with subscriber:
            # Discover sessions as invites arrive
            while len(session_infos) < publishers_count:
                recv_session, payload = await subscriber.receive()
                print(f"[subscriber][mls={mls_enabled}] invited session={recv_session.id}")
                if recv_session.id not in session_infos:
                    session_infos[recv_session.id] = recv_session
                    session_message_counts[recv_session.id] = 0
            # Now process messages for each session
            async def handle_session(session_id: str, info):
                while session_message_counts[session_id] < messages_per_publisher:
                    try:
                        info_recv, payload = await asyncio.wait_for(
                            subscriber.receive(session=session_id), timeout=20 if mls_enabled else 10
                        )
                    except asyncio.TimeoutError:
                        continue
                    if not payload:
                        continue
                    decoded = payload.decode()
                    print(f"[subscriber][mls={mls_enabled}] msg session={session_id}: {decoded}")
                    assert decoded.startswith("msg pub")
                    session_message_counts[session_id] += 1
                    ack = f"ack {session_message_counts[session_id]}".encode()
                    await subscriber.publish(info_recv, ack, info_recv.destination_name)

            await asyncio.gather(
                *[handle_session(sid, sinfo) for sid, sinfo in session_infos.items()]
            )
        for sid, c in session_message_counts.items():
            assert c == messages_per_publisher, (
                f"[mls={mls_enabled}] Session {sid} received {c} messages (expected {messages_per_publisher})"
            )

    async def make_publisher(i: int):
        pub_name = slim_bindings.PyName(org, ns, f"publisher-{i}-mls-{mls_enabled}")
        publisher = await create_slim(pub_name, "secret")
        _ = await publisher.connect({"endpoint": "http://127.0.0.1:12390", "tls": {"insecure": True}})
        # route to subscriber name
        await publisher.set_route(sub_name)
        session_info = await publisher.create_session(
            slim_bindings.PySessionConfiguration.Streaming(
                slim_bindings.PySessionDirection.BIDIRECTIONAL,
                moderator=True,
                topic=sub_name,  # direct session with subscriber
                max_retries=5,
                timeout=datetime.timedelta(seconds=5),
                mls_enabled=mls_enabled,
            )
        )
        # invite subscriber
        await publisher.invite(session_info, sub_name)
        await asyncio.sleep(0.2)
        await ready_event.wait()
        async with publisher:
            for m in range(messages_per_publisher):
                payload = f"msg pub{i} seq{m}".encode()
                print(f"[publisher-{i}][mls={mls_enabled}] send {payload.decode()}")
                await publisher.publish(session_info, payload, sub_name)
                try:
                    while True:
                        session_info, response = await asyncio.wait_for(
                            publisher.receive(session=session_info.id), timeout=15 if mls_enabled else 5
                        )
                        if response and response.decode().startswith("ack"):
                            print(f"[publisher-{i}][mls={mls_enabled}] got {response.decode()}")
                            break
                except asyncio.TimeoutError:
                    raise AssertionError(
                        f"[mls={mls_enabled}] publisher-{i} did not receive ACK for message {m}"
                    )

    sub_task = asyncio.create_task(subscriber_task())
    await asyncio.sleep(5)
    pub_tasks = [asyncio.create_task(make_publisher(i)) for i in range(publishers_count)]
    await asyncio.gather(sub_task, *pub_tasks)

@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:12391"], indirect=True)
async def test_multi_session_single_client_mls(server):
    """MLS-enabled variant: multiple publishers each create a bidirectional session and invite the single subscriber."""

    org = "org"
    ns = "default"
    topic = slim_bindings.PyName(org, ns, "multi-session-mls")
    mls_enabled = True
    publishers_count = 2
    messages_per_publisher = 3
    sub_name = slim_bindings.PyName(org, ns, f"subscriber-mls-{mls_enabled}")
    subscriber = await create_slim(sub_name, "secret")
    _ = await subscriber.connect({"endpoint": "http://127.0.0.1:12391", "tls": {"insecure": True}})

    session_message_counts = {}
    session_infos = {}
    ready_event = asyncio.Event()

    async def subscriber_task():  # noqa: C901
        ready_event.set()
        async with subscriber:
            # Accept invites (one per publisher)
            while len(session_infos) < publishers_count:
                recv_session, _ = await subscriber.receive()
                print(f"[subscriber][mls={mls_enabled}] invited session={recv_session.id}")
                if recv_session.id not in session_infos:
                    session_infos[recv_session.id] = recv_session
                    session_message_counts[recv_session.id] = 0

            async def handle_session(session_id: str, info):
                while session_message_counts[session_id] < messages_per_publisher:
                    try:
                        info_recv, payload = await asyncio.wait_for(
                            subscriber.receive(session=session_id), timeout=25
                        )
                    except asyncio.TimeoutError:
                        continue
                    if not payload:
                        continue
                    decoded = payload.decode()
                    print(f"[subscriber][mls={mls_enabled}] msg session={session_id}: {decoded}")
                    assert decoded.startswith("msg pub")
                    session_message_counts[session_id] += 1
                    ack = f"ack {session_message_counts[session_id]}".encode()
                    await subscriber.publish(info_recv, ack, topic)

            await asyncio.gather(
                *[handle_session(sid, sinfo) for sid, sinfo in session_infos.items()]
            )

        for sid, c in session_message_counts.items():
            assert c == messages_per_publisher, (
                f"[mls={mls_enabled}] Session {sid} received {c} messages (expected {messages_per_publisher})"
            )

    async def make_publisher(i: int):
        pub_name = slim_bindings.PyName(org, ns, f"publisher-{i}-mls-{mls_enabled}")
        publisher = await create_slim(pub_name, "secret")
        _ = await publisher.connect({"endpoint": "http://127.0.0.1:12391", "tls": {"insecure": True}})
        await publisher.set_route(sub_name)
        # Use proper topic separate from participant name
        session_info = await publisher.create_session(
            slim_bindings.PySessionConfiguration.Streaming(
                slim_bindings.PySessionDirection.BIDIRECTIONAL,
                moderator=True,
                topic=topic,
                max_retries=5,
                timeout=datetime.timedelta(seconds=8),
                mls_enabled=True,
            )
        )
        await publisher.invite(session_info, sub_name)
        await asyncio.sleep(0.5)  # allow MLS handshake
        await ready_event.wait()
        async with publisher:
            for m in range(messages_per_publisher):
                payload = f"msg pub{i} seq{m}".encode()
                print(f"[publisher-{i}][mls={mls_enabled}] send {payload.decode()}")
                await publisher.publish(session_info, payload, topic)
                try:
                    while True:
                        session_info, response = await asyncio.wait_for(
                            publisher.receive(session=session_info.id), timeout=20
                        )
                        if response and response.decode().startswith("ack"):
                            print(f"[publisher-{i}][mls={mls_enabled}] got {response.decode()}")
                            break
                except asyncio.TimeoutError:
                    raise AssertionError(
                        f"[mls={mls_enabled}] publisher-{i} did not receive ACK for message {m}"
                    )

    sub_task = asyncio.create_task(subscriber_task())
    await asyncio.sleep(0.3)
    pub_tasks = [asyncio.create_task(make_publisher(i)) for i in range(publishers_count)]
    await asyncio.gather(sub_task, *pub_tasks)
