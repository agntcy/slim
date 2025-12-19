# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime
import pathlib

import pytest

import slim_uniffi_bindings._slim_bindings.slim_bindings as slim_bindings

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


@pytest.mark.skip(
    reason="JWT providers not exposed in new uniffi API - requires create_app_with_jwt() function"
)
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
    sender_name = slim_bindings.Name(
        components=["org", "default", "id_sender"], id=None
    )
    receiver_name = slim_bindings.Name(
        components=["org", "default", "id_receiver"], id=None
    )

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
    session_config = slim_bindings.SessionConfig(
        session_type=slim_bindings.SessionType.POINT_TO_POINT,
        enable_mls=False,
        max_retries=3,
        interval_ms=333,  # ~1 second timeout / 3 retries
        initiator=True,
        metadata={},
    )
    # Create session (auto-waits for establishment)
    if audience == test_audience:
        session_info = await slim_sender.create_session_async(
            session_config, receiver_name
        )
    else:
        # session establishment should timeout due to invalid audience
        with pytest.raises(asyncio.TimeoutError):
            await asyncio.wait_for(
                slim_sender.create_session_async(session_config, receiver_name),
                timeout=3.0,
            )

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
                recv_session = await slim_receiver.listen_for_session_async(None)
                received_msg = await recv_session.get_message_async(None)
                _ctx = received_msg.context
                msg_rcv = received_msg.payload

                # make sure the message is correct
                assert msg_rcv == bytes(pub_msg)

                # reply to the session
                await recv_session.publish_to_async(_ctx, res_msg, None, None)
            except Exception as e:
                print("Error receiving message on slim:", e)

        t = asyncio.create_task(background_task())

        # send a request and expect a response in slim2
        if audience == test_audience:
            # As audience matches, we expect a successful request/reply
            await session_info.publish_async(pub_msg, None, None)
            received_msg = await session_info.get_message_async(None)
            message = received_msg.payload

            # check if the message is correct
            assert message == bytes(res_msg)

            # Wait for task to finish
            await t
    finally:
        # delete sessions (auto-waits for completion)
        if audience == test_audience:
            await slim_sender.delete_session_async(session_info)


def test_invalid_shared_secret_too_short():
    """Test that creating an app with too short shared secret raises an exception."""
    name = slim_bindings.Name(components=["org", "default", "test_app"], id=None)

    # Create app with a secret that's too short (minimum is 32 characters)
    short_secret = "tooshort"

    # Should raise an exception when creating the app
    with pytest.raises(Exception) as exc_info:
        slim_bindings.create_app_with_secret(name, short_secret)

    # Verify the error message mentions the secret being too short
    assert (
        "short" in str(exc_info.value).lower()
        or "invalid" in str(exc_info.value).lower()
    )


def test_invalid_shared_secret_empty():
    """Test that creating an app with empty shared secret raises an exception."""
    name = slim_bindings.Name(components=["org", "default", "test_app"], id=None)

    # Empty secret
    empty_secret = ""

    # Should raise an exception when creating the app
    with pytest.raises(Exception) as exc_info:
        slim_bindings.create_app_with_secret(name, empty_secret)

    # Verify the error message is appropriate
    assert (
        "short" in str(exc_info.value).lower()
        or "invalid" in str(exc_info.value).lower()
    )
