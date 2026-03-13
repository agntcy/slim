// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

using System.Collections.Generic;
using System.Runtime.CompilerServices;
using System.Threading;
using System.Threading.Tasks;
using Google.Protobuf;

namespace Agntcy.Slim.SlimRpc;

/// <summary>
/// Helper methods for streaming protobuf messages over slimrpc streams.
/// </summary>
public static class SlimRpcStreams
{
    /// <summary>
    /// Read protobuf messages from a response stream (unary-to-stream or server side of stream-stream).
    /// </summary>
    /// <typeparam name="T">Protobuf message type (must have a static Parser property).</typeparam>
    /// <param name="reader">The response stream reader.</param>
    /// <param name="cancellationToken">Cancellation token.</param>
    /// <returns>Async enumerable of parsed messages.</returns>
    internal static async IAsyncEnumerable<T> ReadResponseStreamAsync<T>(
        uniffi.slim_bindings.ResponseStreamReader reader,
        [EnumeratorCancellation] CancellationToken cancellationToken = default)
        where T : IMessage<T>, new()
    {
        var parser = new MessageParser<T>(() => new T());
        while (true)
        {
            cancellationToken.ThrowIfCancellationRequested();
            var msg = await reader.NextAsync();
            switch (msg)
            {
                case uniffi.slim_bindings.StreamMessage.Data data:
                    yield return parser.ParseFrom(data.v1);
                    break;
                case uniffi.slim_bindings.StreamMessage.Error err:
                    throw err.v1;
                case uniffi.slim_bindings.StreamMessage.End:
                    yield break;
            }
        }
    }

    /// <summary>
    /// Read protobuf messages from a request stream (stream-unary or stream-stream server side).
    /// </summary>
    /// <typeparam name="T">Protobuf message type (must have a static Parser property).</typeparam>
    /// <param name="stream">The request stream.</param>
    /// <param name="cancellationToken">Cancellation token.</param>
    /// <returns>Async enumerable of parsed messages.</returns>
    internal static async IAsyncEnumerable<T> ReadRequestStreamAsync<T>(
        uniffi.slim_bindings.RequestStream stream,
        [EnumeratorCancellation] CancellationToken cancellationToken = default)
        where T : IMessage<T>, new()
    {
        var parser = new MessageParser<T>(() => new T());
        while (true)
        {
            cancellationToken.ThrowIfCancellationRequested();
            var msg = await stream.NextAsync();
            switch (msg)
            {
                case uniffi.slim_bindings.StreamMessage.Data data:
                    yield return parser.ParseFrom(data.v1);
                    break;
                case uniffi.slim_bindings.StreamMessage.Error err:
                    throw err.v1;
                case uniffi.slim_bindings.StreamMessage.End:
                    yield break;
            }
        }
    }

    /// <summary>
    /// Read protobuf messages from a bidirectional stream (client side of stream-stream).
    /// </summary>
    /// <typeparam name="T">Protobuf message type (must have a static Parser property).</typeparam>
    /// <param name="bidi">The bidirectional stream handler.</param>
    /// <param name="cancellationToken">Cancellation token.</param>
    /// <returns>Async enumerable of parsed messages.</returns>
    internal static async IAsyncEnumerable<T> ReadBidiStreamAsync<T>(
        uniffi.slim_bindings.BidiStreamHandler bidi,
        [EnumeratorCancellation] CancellationToken cancellationToken = default)
        where T : IMessage<T>, new()
    {
        var parser = new MessageParser<T>(() => new T());
        while (true)
        {
            cancellationToken.ThrowIfCancellationRequested();
            var msg = await bidi.RecvAsync();
            switch (msg)
            {
                case uniffi.slim_bindings.StreamMessage.Data data:
                    yield return parser.ParseFrom(data.v1);
                    break;
                case uniffi.slim_bindings.StreamMessage.Error err:
                    throw err.v1;
                case uniffi.slim_bindings.StreamMessage.End:
                    yield break;
            }
        }
    }
}
