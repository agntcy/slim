// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc;

import io.agntcy.slim.bindings.ResponseStreamReader;
import io.agntcy.slim.bindings.RpcException;
import io.agntcy.slim.bindings.StreamMessage;

import java.util.function.Function;

public final class ClientResponseStream<RespT> {
    private final ResponseStreamReader inner;
    private final Function<byte[], RespT> parser;

    public ClientResponseStream(ResponseStreamReader inner, Function<byte[], RespT> parser) {
        this.inner = inner;
        this.parser = parser;
    }

    public static <RespT> ClientResponseStream<RespT> create(
            ResponseStreamReader reader,
            Function<byte[], RespT> parser
    ) {
        return new ClientResponseStream<>(reader, parser);
    }

    public RespT recv() throws RpcException {
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
