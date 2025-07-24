# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime

import pytest

import slim_bindings

keys_folder = "./tests/testdata"

test_audience = ["test.audience"]


def create_slim(
    organization,
    namespace,
    agent_type,
    private_key,
    private_key_algorithm,
    public_key,
    public_key_algorithm,
    wrong_audience=None,
):
    private_key = slim_bindings.PyKey(
        algorithm=private_key_algorithm,
        format=slim_bindings.PyKeyFormat.Pem,
        key=slim_bindings.PyKeyData.File(path=private_key),
    )

    public_key = slim_bindings.PyKey(
        algorithm=public_key_algorithm,
        format=slim_bindings.PyKeyFormat.Pem,
        key=slim_bindings.PyKeyData.File(path=public_key),
    )

    provider = slim_bindings.PyIdentityProvider.Jwt(
        private_key=private_key,
        duration=datetime.timedelta(seconds=60),
        issuer="test-issuer",
        audience=test_audience,
        subject=agent_type,
    )
    verifier = slim_bindings.PyIdentityVerifier.Jwt(
        public_key=public_key,
        issuer="test-issuer",
        audience=wrong_audience or test_audience,
        require_iss=True,
        require_aud=True,
    )

    return slim_bindings.Slim.new(
        organization, namespace, agent_type, provider, verifier
    )


@pytest.mark.asyncio
@pytest.mark.parametrize("server", ["127.0.0.1:52345"], indirect=True)
@pytest.mark.parametrize("audience", [test_audience, ["wrong.audience"]])
async def test_identity_verification(server, audience):
    org = "org"
    ns = "default"
    sender = "sender"

    # Keys used for signing JWTs of sender
    private_key_sender = f"{keys_folder}/ec256.pem"
    public_key_sender = f"{keys_folder}/ec256-public.pem"
    algorithm_sender = slim_bindings.PyAlgorithm.ES256

    # Keys used for signing JWTs of receiver
    private_key_receiver = f"{keys_folder}/ec384.pem"
    public_key_receiver = f"{keys_folder}/ec384-public.pem"
    algorithm_receiver = slim_bindings.PyAlgorithm.ES384

    # create new slim object. note that the verifier will use the public key of the receiver
    # to verify the JWT of the reply message
    slim_sender = await create_slim(
        org,
        ns,
        sender,
        private_key_sender,
        algorithm_sender,
        public_key_receiver,
        algorithm_receiver,
    )

    # Connect to the service and subscribe for the local name
    _ = await slim_sender.connect(
        {"endpoint": "http://127.0.0.1:52345", "tls": {"insecure": True}}
    )

    # create second local agent. note that the receiver will use the public key of the sender
    # to verify the JWT of the request message
    receiver = "receiver"
    slim_receiver = await create_slim(
        org,
        ns,
        receiver,
        private_key_receiver,
        algorithm_receiver,
        public_key_sender,
        algorithm_sender,
        audience,
    )

    # Connect to SLIM server
    _ = await slim_receiver.connect(
        {"endpoint": "http://127.0.0.1:52345", "tls": {"insecure": True}}
    )

    # set route
    await slim_sender.set_route(org, ns, receiver)

    # create request/reply session with default config
    session_info = await slim_sender.create_session(
        slim_bindings.PySessionConfiguration.FireAndForget(
            timeout=datetime.timedelta(seconds=1), max_retries=3, sticky=False
        )
    )

    # messages
    pub_msg = str.encode("thisistherequest")
    res_msg = str.encode("thisistheresponse")

    # Test with reply
    async with slim_sender, slim_receiver:
        # create background task for slim_receiver
        async def background_task():
            try:
                # wait for message from any new session
                recv_session, _ = await slim_receiver.receive()

                # receive message from session
                recv_session, msg_rcv = await slim_receiver.receive(
                    session=recv_session.id
                )

                # make sure the message is correct
                assert msg_rcv == bytes(pub_msg)

                # reply to the session
                await slim_receiver.publish_to(recv_session, res_msg)
            except Exception as e:
                print("Error receiving message on slim1:", e)

        t = asyncio.create_task(background_task())

        # send a request and expect a response in slim2
        if audience == test_audience:
            # As audience matches, we expect a successful request/reply
            session_info, message = await slim_sender.request_reply(
                session_info, pub_msg, org, ns, receiver
            )

            # check if the message is correct
            assert message == bytes(res_msg)

            # Wait for task to finish
            await t
        else:
            # expect an exception due to audience mismatch
            with pytest.raises(asyncio.TimeoutError):
                session_info, message = await slim_sender.request_reply(
                    session_info,
                    pub_msg,
                    org,
                    ns,
                    receiver,
                    timeout=datetime.timedelta(seconds=3),
                )

            # cancel the background task
            t.cancel()

            # wait for task to finish
            try:
                await t
            except asyncio.CancelledError:
                print("Background task cancelled as expected.")
