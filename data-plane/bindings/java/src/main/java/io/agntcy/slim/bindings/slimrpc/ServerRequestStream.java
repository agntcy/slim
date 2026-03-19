// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc;

import io.agntcy.slim.bindings.ResponseSink;
import io.agntcy.slim.bindings.RpcException;

import java.util.function.Function;

public interface ServerRequestStream<RespT> {
    void send(RespT response) throws RpcException;

    static <RespT> ServerRequestStream<RespT> create(
            ResponseSink sink,
            Function<RespT, byte[]> serializer
    ) {
        return new ServerRequestStreamImpl<>(sink, serializer);
    }
}

final class ServerRequestStreamImpl<RespT> implements ServerRequestStream<RespT> {
    private final ResponseSink inner;
    private final Function<RespT, byte[]> serializer;

    ServerRequestStreamImpl(ResponseSink inner, Function<RespT, byte[]> serializer) {
        this.inner = inner;
        this.serializer = serializer;
    }

    @Override
    public void send(RespT response) throws RpcException {
        inner.sendAsync(serializer.apply(response)).join();
    }
}
