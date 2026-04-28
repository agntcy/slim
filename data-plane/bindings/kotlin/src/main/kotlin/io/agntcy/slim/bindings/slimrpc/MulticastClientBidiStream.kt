// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc

import io.agntcy.slim.bindings.MulticastBidiStreamHandler
import io.agntcy.slim.bindings.MulticastStreamMessage

class MulticastClientBidiStream<ReqT, RespT>(
    private val inner: MulticastBidiStreamHandler,
    private val serializer: (ReqT) -> ByteArray,
    private val parser: (ByteArray) -> RespT
) {
    suspend fun send(request: ReqT) {
        inner.sendAsync(serializer(request))
    }

    suspend fun closeSend() {
        inner.closeSendAsync()
    }

    suspend fun recv(): MulticastStreamMessage {
        return inner.recvAsync()
    }
}
