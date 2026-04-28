// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc

import io.agntcy.slim.bindings.ResponseSink

interface ServerRequestStream<RespT> {
    suspend fun send(response: RespT)

    companion object {
        fun <RespT> create(
            sink: ResponseSink,
            serializer: (RespT) -> ByteArray
        ): ServerRequestStream<RespT> {
            return ServerRequestStreamImpl(sink, serializer)
        }
    }
}

internal class ServerRequestStreamImpl<RespT>(
    private val inner: ResponseSink,
    private val serializer: (RespT) -> ByteArray
) : ServerRequestStream<RespT> {

    override suspend fun send(response: RespT) {
        inner.sendAsync(serializer(response))
    }
}
