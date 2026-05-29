// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings.slimrpc;

import io.agntcy.slim.bindings.MulticastResponseReader;
import io.agntcy.slim.bindings.MulticastStreamMessage;

import java.util.function.Function;

/**
 * Typed wrapper around {@link MulticastResponseReader} that deserializes
 * multicast response items.
 *
 * <p>Used by generated group client stubs for multicast unary-unary and
 * unary-stream patterns.
 *
 * @param <RespT> The deserialized response type.
 */
public final class MulticastResponseStream<RespT> {
    private final MulticastResponseReader inner;
    private final Function<byte[], RespT> parser;

    public MulticastResponseStream(MulticastResponseReader inner, Function<byte[], RespT> parser) {
        this.inner = inner;
        this.parser = parser;
    }

    /**
     * Pull the next message from the multicast stream (blocking).
     *
     * @return The next {@link MulticastStreamMessage}. Callers should
     *         pattern-match on {@code Data}, {@code Error}, and {@code End}.
     */
    public MulticastStreamMessage next() {
        return inner.nextAsync().join();
    }
}