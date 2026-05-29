// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc

import io.agntcy.slim.bindings.BidiStreamHandler
import io.agntcy.slim.bindings.StreamMessage

class ClientBidiStream<ReqT>(
    private val inner: BidiStreamHandler,
    private val serializer: (ReqT) -> ByteArray
) {
    suspend fun send(request: ReqT) {
        inner.sendAsync(serializer(request))
    }

    suspend fun closeSend() {
        inner.closeSendAsync()
    }

    suspend fun recv(): StreamMessage {
        return inner.recvAsync()
    }
}
