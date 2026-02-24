# Copyright AGNTCY Contributors (https://github.com/agntcy)
# SPDX-License-Identifier: Apache-2.0

import asyncio
import datetime
import pathlib

import pytest

import slim_bindings

# Shared certs: data-plane/testdata/certs (from tests/ -> bindings/python -> bindings -> data-plane)
_keys_dir = pathlib.Path(__file__).resolve().parent.parent.parent.parent / "testdata" / "certs"
keys_folder = str(_keys_dir)

test_audience = ["test.audience"]


def create_slim(
    name: slim_bindings.Name,
    private_key,
    private_key_algorithm,
    public_key,
    public_key_algorithm,
    wrong_audience=None,
):
    """Construct an App instance with a JWT identity provider/verifier.

    Args:
        name: Name identifying this local app (used as JWT subject).
        private_key: Path to PEM private key used for signing outbound tokens.
        private_key_algorithm: Algorithm matching the private key type (e.g. ES256).
        public_key: Path to PEM public key used to verify the peer's tokens.
        public_key_algorithm: Algorithm matching the peer public key type.
        wrong_audience: Optional override audience list to force verification failure.
                        If None, uses the shared test_audience (success path).

    Returns:
        App: A configured App instance.
    """

    # Build signing key config (private key for encoding)
    private_key_config = slim_bindings.JwtKeyConfig(
        algorithm=private_key_algorithm,
        format=slim_bindings.JwtKeyFormat.PEM,
        key=slim_bindings.JwtKeyData.FILE(path=private_key),  # type: ignore
    )

    # Build verification key config (public key for decoding)
    public_key_config = slim_bindings.JwtKeyConfig(
        algorithm=public_key_algorithm,
        format=slim_bindings.JwtKeyFormat.PEM,
        key=slim_bindings.JwtKeyData.FILE(path=public_key),  # type: ignore
    )

    # Create provider config (for signing outbound tokens)
    provider_config = slim_bindings.IdentityProviderConfig.JWT(
        config=slim_bindings.ClientJwtAuth(
            key=slim_bindings.JwtKeyType.ENCODING(key=private_key_config),  # type: ignore
            audience=test_audience,
            issuer="test-issuer",
            subject=f"{name}",
            duration=datetime.timedelta(seconds=60),
        )
    )

    # Create verifier config (for verifying inbound tokens)
    verifier_config = slim_bindings.IdentityVerifierConfig.JWT(
        config=slim_bindings.JwtAuth(
            key=slim_bindings.JwtKeyType.DECODING(key=public_key_config),  # type: ignore
            audience=wrong_audience or test_audience,
            issuer="test-issuer",
            subject=None,
            duration=datetime.timedelta(seconds=60),
        )
    )

    # Create and return the app
    return slim_bindings.App(name, provider_config, verifier_config)  # type: ignore[arg-type]


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
        slim_bindings.JwtAlgorithm.ES256
    )  # Curves/selections align with private key

    # Keys used for signing JWTs of receiver
    private_key_receiver = f"{keys_folder}/ec384.pem"  # Receiver's signing key (ES384)
    public_key_receiver = (
        f"{keys_folder}/ec384-public.pem"  # Public half used by sender to verify
    )
    algorithm_receiver = slim_bindings.JwtAlgorithm.ES384

    # create new app object. note that the verifier will use the public key of the receiver
    # to verify the JWT of the reply message
    app_sender = create_slim(
        sender_name,
        private_key_sender,
        algorithm_sender,
        public_key_receiver,
        algorithm_receiver,
    )

    # create second local app. note that the receiver will use the public key of the sender
    # to verify the JWT of the request message
    app_receiver = create_slim(
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
        interval=datetime.timedelta(milliseconds=333),
        metadata={},
    )
    # Create session (returns immediately with session_context)
    session_context = await app_sender.create_session_async(
        session_config, receiver_name
    )

    # Wait for session establishment based on audience
    if audience == test_audience:
        await session_context.completion.wait_async()
    else:
        # session establishment should timeout due to invalid audience
        with pytest.raises(slim_bindings.SlimError.SessionError):
            await asyncio.wait_for(
                session_context.completion.wait_async(),
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

            recv_session_ctx = None
            try:
                recv_session_ctx = await app_receiver.listen_for_session_async(None)
                received_msg = await recv_session_ctx.get_message_async(None)
                _ctx = received_msg.context
                msg_rcv = received_msg.payload

                # make sure the message is correct
                assert msg_rcv == bytes(pub_msg)

                # reply to the session
                await recv_session_ctx.publish_to_async(_ctx, res_msg, None, None)
            except Exception as e:
                print("Error receiving message on app:", e)

        t = asyncio.create_task(background_task())

        # send a request and expect a response
        if audience == test_audience:
            # As audience matches, we expect a successful request/reply
            handle = await session_context.session.publish_async(pub_msg, None, None)
            await handle.wait_async()
            received_msg = await session_context.session.get_message_async(None)
            message = received_msg.payload

            # check if the message is correct
            assert message == bytes(res_msg)

            # Wait for task to finish (and surface any exceptions)
            _ = await t
    finally:
        # delete sessions
        if audience == test_audience:
            h = await app_sender.delete_session_async(session_context.session)
            await h.wait_async()


def test_invalid_shared_secret_too_short():
    """Test that creating an app with too short shared secret raises an exception."""
    name = slim_bindings.Name("org", "default", "test_app")

    # Create app with a secret that's too short (minimum is 32 characters)
    short_secret = "tooshort"

    # Get the global service
    service = slim_bindings.get_global_service()

    # Should raise an exception when creating the app
    with pytest.raises(Exception) as exc_info:
        service.create_app_with_secret(name, short_secret)

    # Verify the error message mentions the secret being too short
    assert (
        "short" in str(exc_info.value).lower()
        or "invalid" in str(exc_info.value).lower()
    )


def test_invalid_shared_secret_empty():
    """Test that creating an app with empty shared secret raises an exception."""
    name = slim_bindings.Name("org", "default", "test_app")

    # Empty secret
    empty_secret = ""

    # Get the global service
    service = slim_bindings.get_global_service()

    # Should raise an exception when creating the app
    with pytest.raises(Exception) as exc_info:
        service.create_app_with_secret(name, empty_secret)

    # Verify the error message is appropriate
    assert (
        "short" in str(exc_info.value).lower()
        or "invalid" in str(exc_info.value).lower()
    )
