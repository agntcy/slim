// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc

import com.google.protobuf.InvalidProtocolBufferException
import com.google.protobuf.Parser
import io.agntcy.slim.bindings.RpcCode
import io.agntcy.slim.bindings.RpcException
import java.util.concurrent.CompletionException

fun <T> parse(data: ByteArray, parser: Parser<T>): T {
    try {
        return parser.parseFrom(data)
    } catch (e: InvalidProtocolBufferException) {
        throw CompletionException(e)
    }
}

fun toRpcException(error: Throwable, defaultCode: RpcCode): RpcException {
    val unwrapped = unwrap(error)
    if (unwrapped is RpcException) {
        return unwrapped
    }
    val message = unwrapped.message?.takeIf { it.isNotBlank() } ?: unwrapped.toString()
    return RpcException.Rpc(defaultCode, message, null)
}

fun unwrap(error: Throwable): Throwable {
    if (error is CompletionException && error.cause != null) {
        return error.cause!!
    }
    return error
}
