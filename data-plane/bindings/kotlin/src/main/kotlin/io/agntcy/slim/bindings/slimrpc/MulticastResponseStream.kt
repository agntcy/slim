// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc

import io.agntcy.slim.bindings.MulticastResponseReader
import io.agntcy.slim.bindings.MulticastStreamMessage

class MulticastResponseStream<RespT>(
    private val inner: MulticastResponseReader,
    private val parser: (ByteArray) -> RespT
) {
    suspend fun next(): MulticastStreamMessage {
        return inner.nextAsync()
    }
}
