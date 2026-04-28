// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc

import io.agntcy.slim.bindings.RequestStream
import io.agntcy.slim.bindings.ResponseSink
import io.agntcy.slim.bindings.StreamMessage

interface ServerBidiStream<ReqT, RespT> {
    suspend fun recv(): ReqT?
    suspend fun send(response: RespT)

    companion object {
        fun <ReqT, RespT> create(
            stream: RequestStream,
            sink: ResponseSink,
            parser: (ByteArray) -> ReqT,
            serializer: (RespT) -> ByteArray
        ): ServerBidiStream<ReqT, RespT> {
            return ServerBidiStreamImpl(stream, sink, parser, serializer)
        }
    }
}

internal class ServerBidiStreamImpl<ReqT, RespT>(
    private val stream: RequestStream,
    private val sink: ResponseSink,
    private val parser: (ByteArray) -> ReqT,
    private val serializer: (RespT) -> ByteArray
) : ServerBidiStream<ReqT, RespT> {

    override suspend fun recv(): ReqT? {
        return when (val message = stream.nextAsync()) {
            is StreamMessage.End -> null
            is StreamMessage.Error -> throw message.v1
            is StreamMessage.Data -> parser(message.v1)
        }
    }

    override suspend fun send(response: RespT) {
        sink.sendAsync(serializer(response))
    }
}
