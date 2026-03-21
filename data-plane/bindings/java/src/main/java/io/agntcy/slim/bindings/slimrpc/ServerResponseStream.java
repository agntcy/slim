// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc;

import io.agntcy.slim.bindings.RequestStream;
import io.agntcy.slim.bindings.RpcException;
import io.agntcy.slim.bindings.StreamMessage;

import java.util.function.Function;

public interface ServerResponseStream<ReqT> {
    ReqT recv() throws RpcException;

    static <ReqT> ServerResponseStream<ReqT> create(
            RequestStream stream,
            Function<byte[], ReqT> parser
    ) {
        return new ServerResponseStreamImpl<>(stream, parser);
    }
}

final class ServerResponseStreamImpl<ReqT> implements ServerResponseStream<ReqT> {
    private final RequestStream inner;
    private final Function<byte[], ReqT> parser;

    ServerResponseStreamImpl(RequestStream inner, Function<byte[], ReqT> parser) {
        this.inner = inner;
        this.parser = parser;
    }

    @Override
    public ReqT recv() throws RpcException {
        StreamMessage message = inner.nextAsync().join();
        if (message instanceof StreamMessage.End) {
            return null;
        }
        if (message instanceof StreamMessage.Error err) {
            throw err.v1();
        }
        if (message instanceof StreamMessage.Data data) {
            return parser.apply(data.v1());
        }
        throw new IllegalStateException("unknown stream message type");
    }
}
