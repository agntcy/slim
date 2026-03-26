// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc;

import io.agntcy.slim.bindings.RequestStreamWriter;
import io.agntcy.slim.bindings.RpcException;

import java.util.function.Function;

public final class ClientRequestStream<ReqT, ResT> {
    private final RequestStreamWriter inner;
    private final Function<ReqT, byte[]> serializer;
    private final Function<byte[], ResT> parser;

    public ClientRequestStream(RequestStreamWriter inner, Function<ReqT, byte[]> serializer, Function<byte[], ResT> parser) {
        this.inner = inner;
        this.serializer = serializer;
        this.parser = parser;
    }

    public void send(ReqT request) throws RpcException {
        inner.sendAsync(serializer.apply(request)).join();
    }

    public ResT finalizeStream() throws RpcException {
        return parser.apply(inner.finalizeStreamAsync().join());
    }
}
