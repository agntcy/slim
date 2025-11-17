# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime
import pathlib

import pytest

import slim_bindings

keys_folder = f"{pathlib.Path(__file__).parent.resolve()}/testdata"

test_audience = ["test.audience"]


def create_slim(
    name: slim_bindings.Name,
    private_key,
    private_key_algorithm,
    public_key,
    public_key_algorithm,
    wrong_audience=None,
):
    """Asynchronously construct a Slim instance with a JWT identity provider/verifier.

    Args:
        name: Name identifying this local app (used as JWT subject).
        private_key: Path to PEM private key used for signing outbound tokens.
        private_key_algorithm: Algorithm matching the private key type (e.g. ES256).
        public_key: Path to PEM public key used to verify the peer's tokens.
        public_key_algorithm: Algorithm matching the peer public key type.
        wrong_audience: Optional override audience list to force verification failure.
                        If None, uses the shared test_audience (success path).

    Returns:
        Awaitable[Slim]: A coroutine yielding a configured Slim instance.
    """
    # Build signing key object (private)
    private_key = slim_bindings.Key(
        algorithm=private_key_algorithm,
        format=slim_bindings.KeyFormat.Pem,
        key=slim_bindings.KeyData.File(path=private_key),
    )

    public_key = slim_bindings.Key(
        algorithm=public_key_algorithm,
        format=slim_bindings.KeyFormat.Pem,
        key=slim_bindings.KeyData.File(path=public_key),
    )

    provider = slim_bindings.IdentityProvider.Jwt(
        private_key=private_key,
        duration=datetime.timedelta(seconds=60),
        issuer="test-issuer",
        audience=test_audience,
        subject=f"{name}",
    )

    verifier = slim_bindings.IdentityVerifier.Jwt(
        public_key=public_key,
        issuer="test-issuer",
        audience=wrong_audience or test_audience,
        require_iss=True,
        require_aud=True,
    )

    return slim_bindings.Slim(name, provider, verifier, local_service=False)


@pytest.mark.asyncio
@pytest.mark.parametrize("server", [None], indirect=True)
@pytest.mark.parametrize("audience", [test_audience, ["wrong.audience"]])
async def test_identity_verification(server, audience):
    """End-to-end JWT identity verification test.

    Parametrized:
        audience:
            - Matching audience list (expects successful request/reply)
            - Wrong audience list (expects receive timeout / verification failure)

    Flow:
        1. Create sender & receiver Slim instances with distinct EC key pairs.
        2. Cross-wire each instance: each verifier trusts the other's public key.
        3. Establish route sender -> receiver.
        4. Sender creates PointToPoint session and publishes a request.
        5. Receiver listens, validates payload, replies.
        6. Validate response only when audience matches; otherwise expect timeout.

    Assertions:
        - Payload integrity on both directions when audience matches.
        - Proper exception/timeout on audience mismatch.
    """
    sender_name = slim_bindings.Name("org", "default", "id_sender")
    receiver_name = slim_bindings.Name("org", "default", "id_receiver")

    # Keys used for signing JWTs of sender
    private_key_sender = f"{keys_folder}/ec256.pem"  # Sender's signing key (ES256)
    public_key_sender = (
        f"{keys_folder}/ec256-public.pem"  # Public half used by receiver to verify
    )
    algorithm_sender = (
        slim_bindings.Algorithm.ES256
    )  # Curves/selections align with private key

    # Keys used for signing JWTs of receiver
    private_key_receiver = f"{keys_folder}/ec384.pem"  # Receiver's signing key (ES384)
    public_key_receiver = (
        f"{keys_folder}/ec384-public.pem"  # Public half used by sender to verify
    )
    algorithm_receiver = slim_bindings.Algorithm.ES384

    # create new slim object. note that the verifier will use the public key of the receiver
    # to verify the JWT of the reply message
    slim_sender = create_slim(
        sender_name,
        private_key_sender,
        algorithm_sender,
        public_key_receiver,
        algorithm_receiver,
    )

    # create second local app. note that the receiver will use the public key of the sender
    # to verify the JWT of the request message
    slim_receiver = create_slim(
        receiver_name,
        private_key_receiver,
        algorithm_receiver,
        public_key_sender,
        algorithm_sender,
        audience,
    )

    # Create PointToPoint session
    session_config = slim_bindings.SessionConfiguration.PointToPoint(
        max_retries=3,
        timeout=datetime.timedelta(seconds=1),
    )
    session_info, completion_handle = await slim_sender.create_session(
        receiver_name, session_config
    )

    # wait for session establishment if the audience is valid
    if audience == test_audience:
        await completion_handle
    else:
        # session establishment should timeout due to invalid audience
        with pytest.raises(asyncio.TimeoutError):
            await asyncio.wait_for(completion_handle, timeout=3.0)

    # messages
    pub_msg = str.encode("thisistherequest")
    res_msg = str.encode("thisistheresponse")

    # Test with reply
    try:
        # create background task for slim_receiver
        async def background_task():
            """Receiver side:
            - Wait for inbound session
            - Receive request
            - Reply with response payload
            """

            recv_session = None
            try:
                recv_session = await slim_receiver.listen_for_session()
                (
                    _ctx,
                    msg_rcv,
                ) = await recv_session.get_message()  # (_ctx carries reply addressing)

                # make sure the message is correct
                assert msg_rcv == bytes(pub_msg)

                # reply to the session
                await recv_session.publish_to(_ctx, res_msg)
            except Exception as e:
                print("Error receiving message on slim:", e)

        t = asyncio.create_task(background_task())

        # send a request and expect a response in slim2
        if audience == test_audience:
            # As audience matches, we expect a successful request/reply
            await session_info.publish(pub_msg)
            _ctx2, message = await session_info.get_message()

            # check if the message is correct
            assert message == bytes(res_msg)

            # Wait for task to finish
            await t
    finally:
        # delete sessions
        h = await slim_sender.delete_session(session_info)
        await h
