// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc;

import io.agntcy.slim.bindings.MulticastBidiStreamHandler;
import io.agntcy.slim.bindings.MulticastStreamMessage;
import io.agntcy.slim.bindings.RpcException;

import java.util.function.Function;

/**
 * Typed wrapper around {@link MulticastBidiStreamHandler} that handles
 * serialization of requests and provides access to multicast response messages.
 *
 * <p>Used by generated group client stubs for multicast stream-unary and
 * stream-stream patterns.
 *
 * @param <ReqT>  The request type (serialized to bytes before sending).
 * @param <RespT> The response type (for type-safety at the call site;
 *                actual deserialization is done by the caller when
 *                pattern-matching on {@link MulticastStreamMessage.Data}).
 */
public final class MulticastClientBidiStream<ReqT, RespT> {
    private final MulticastBidiStreamHandler inner;
    private final Function<ReqT, byte[]> serializer;
    private final Function<byte[], RespT> parser;

    public MulticastClientBidiStream(
            MulticastBidiStreamHandler inner,
            Function<ReqT, byte[]> serializer,
            Function<byte[], RespT> parser) {
        this.inner = inner;
        this.serializer = serializer;
        this.parser = parser;
    }

    /**
     * Send a request to all group members (blocking).
     *
     * @param request The request to send.
     * @throws RpcException if the send fails.
     */
    public void send(ReqT request) throws RpcException {
        inner.sendAsync(serializer.apply(request)).join();
    }

    /**
     * Signal that no more requests will be sent (blocking).
     *
     * @throws RpcException if closing the send side fails.
     */
    public void closeSend() throws RpcException {
        inner.closeSendAsync().join();
    }

    /**
     * Pull the next message from the multicast response stream (blocking).
     *
     * @return The next {@link MulticastStreamMessage}. Callers should
     *         pattern-match on {@code Data}, {@code Error}, and {@code End}.
     */
    public MulticastStreamMessage recv() {
        return inner.recvAsync().join();
    }
}