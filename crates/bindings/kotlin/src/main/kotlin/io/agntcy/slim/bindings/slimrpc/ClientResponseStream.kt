// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc

import io.agntcy.slim.bindings.ResponseStreamReader
import io.agntcy.slim.bindings.StreamMessage

class ClientResponseStream<RespT>(
    private val inner: ResponseStreamReader,
    private val parser: (ByteArray) -> RespT
) {
    suspend fun recv(): RespT? {
        return when (val message = inner.nextAsync()) {
            is StreamMessage.End -> null
            is StreamMessage.Error -> throw message.v1
            is StreamMessage.Data -> parser(message.v1)
        }
    }

    companion object {
        fun <RespT> create(
            reader: ResponseStreamReader,
            parser: (ByteArray) -> RespT
        ): ClientResponseStream<RespT> {
            return ClientResponseStream(reader, parser)
        }
    }
}
