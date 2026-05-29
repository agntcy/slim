// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc

import io.agntcy.slim.bindings.RequestStream
import io.agntcy.slim.bindings.StreamMessage

interface ServerResponseStream<ReqT> {
    suspend fun recv(): ReqT?

    companion object {
        fun <ReqT> create(
            stream: RequestStream,
            parser: (ByteArray) -> ReqT
        ): ServerResponseStream<ReqT> {
            return ServerResponseStreamImpl(stream, parser)
        }
    }
}

internal class ServerResponseStreamImpl<ReqT>(
    private val inner: RequestStream,
    private val parser: (ByteArray) -> ReqT
) : ServerResponseStream<ReqT> {

    override suspend fun recv(): ReqT? {
        return when (val message = inner.nextAsync()) {
            is StreamMessage.End -> null
            is StreamMessage.Error -> throw message.v1
            is StreamMessage.Data -> parser(message.v1)
        }
    }
}
