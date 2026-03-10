// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

using System.Collections.Generic;

namespace Agntcy.Slim.SlimRpc;

/// <summary>
/// Context passed to slimrpc server handlers, providing metadata, deadline, and session information.
/// </summary>
public sealed class SlimRpcContext
{
    private readonly uniffi.slim_bindings.Context _inner;

    internal SlimRpcContext(uniffi.slim_bindings.Context inner)
    {
        _inner = inner;
    }

    /// <summary>
    /// Create a SlimRpcContext from the raw FFI Context.
    /// </summary>
    internal static SlimRpcContext FromContext(uniffi.slim_bindings.Context context)
    {
        return new SlimRpcContext(context);
    }

    /// <summary>
    /// Get the RPC session metadata.
    /// </summary>
    public IReadOnlyDictionary<string, string> Metadata => _inner.Metadata();

    /// <summary>
    /// Get the deadline for this RPC call.
    /// </summary>
    public System.DateTime Deadline => _inner.Deadline();

    /// <summary>
    /// Check if the deadline has been exceeded.
    /// </summary>
    public bool IsDeadlineExceeded => _inner.IsDeadlineExceeded();

    /// <summary>
    /// Get the remaining time until deadline.
    /// Returns TimeSpan.Zero if the deadline has already passed.
    /// </summary>
    public System.TimeSpan RemainingTime => _inner.RemainingTime();

    /// <summary>
    /// Get the session ID.
    /// </summary>
    public string SessionId => _inner.SessionId();
}
