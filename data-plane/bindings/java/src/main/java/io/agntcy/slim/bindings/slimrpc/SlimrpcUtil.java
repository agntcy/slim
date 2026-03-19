// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc;

import io.agntcy.slim.bindings.RpcCode;
import io.agntcy.slim.bindings.RpcException;

import com.google.protobuf.InvalidProtocolBufferException;
import com.google.protobuf.Parser;

import java.util.concurrent.CompletionException;

public final class SlimrpcUtil {
    private SlimrpcUtil() {}

    public static <T> T parse(byte[] data, Parser<T> parser) {
        try {
            return parser.parseFrom(data);
        } catch (InvalidProtocolBufferException e) {
            throw new CompletionException(e);
        }
    }

    public static RpcException toRpcException(Throwable error, RpcCode defaultCode) {
        Throwable unwrapped = unwrap(error);
        if (unwrapped instanceof RpcException rpc) {
            return rpc;
        }
        String message = unwrapped.getMessage();
        if (message == null || message.isBlank()) {
            message = unwrapped.toString();
        }
        return new RpcException.Rpc(defaultCode, message, null);
    }

    public static Throwable unwrap(Throwable error) {
        if (error instanceof CompletionException ce && ce.getCause() != null) {
            return ce.getCause();
        }
        return error;
    }
}
