// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc;

import io.agntcy.slim.bindings.RequestStream;
import io.agntcy.slim.bindings.ResponseSink;
import io.agntcy.slim.bindings.RpcException;
import io.agntcy.slim.bindings.StreamMessage;

import java.util.function.Function;

public interface ServerBidiStream<ReqT, RespT> {
    ReqT recv() throws RpcException;
    void send(RespT response) throws RpcException;

    static <ReqT, RespT> ServerBidiStream<ReqT, RespT> create(
            RequestStream stream,
            ResponseSink sink,
            Function<byte[], ReqT> parser,
            Function<RespT, byte[]> serializer
    ) {
        return new ServerBidiStreamImpl<>(stream, sink, parser, serializer);
    }
}

final class ServerBidiStreamImpl<ReqT, RespT> implements ServerBidiStream<ReqT, RespT> {
    private final RequestStream stream;
    private final ResponseSink sink;
    private final Function<byte[], ReqT> parser;
    private final Function<RespT, byte[]> serializer;

    ServerBidiStreamImpl(
            RequestStream stream,
            ResponseSink sink,
            Function<byte[], ReqT> parser,
            Function<RespT, byte[]> serializer
    ) {
        this.stream = stream;
        this.sink = sink;
        this.parser = parser;
        this.serializer = serializer;
    }

    @Override
    public ReqT recv() throws RpcException {
        StreamMessage message = stream.nextAsync().join();
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

    @Override
    public void send(RespT response) throws RpcException {
        sink.sendAsync(serializer.apply(response)).join();
    }
}
