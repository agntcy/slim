// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc

import io.agntcy.slim.bindings.RequestStreamWriter

class ClientRequestStream<ReqT, ResT>(
    private val inner: RequestStreamWriter,
    private val serializer: (ReqT) -> ByteArray,
    private val parser: (ByteArray) -> ResT
) {
    suspend fun send(request: ReqT) {
        inner.sendAsync(serializer(request))
    }

    suspend fun finalizeStream(): ResT {
        return parser(inner.finalizeStreamAsync())
    }
}
