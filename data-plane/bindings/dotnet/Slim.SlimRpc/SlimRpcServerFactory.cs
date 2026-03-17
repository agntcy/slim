// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

using Agntcy.Slim;

namespace Agntcy.Slim.SlimRpc;

/// <summary>
/// Factory for creating slimrpc servers from SlimApp.
/// </summary>
public static class SlimRpcServerFactory
{
    /// <summary>
    /// Create an RPC server for the given base name.
    /// </summary>
    /// <param name="app">The Slim application.</param>
    /// <param name="baseName">The base name for the server.</param>
    /// <param name="connectionId">Optional connection ID to use.</param>
    /// <returns>A server for handling RPC requests.</returns>
    public static uniffi.slim_bindings.Server CreateServer(
        SlimApp app,
        SlimName baseName,
        ulong? connectionId = null)
    {
        return uniffi.slim_bindings.Server.NewWithConnection(app._inner, baseName._inner, connectionId);
    }
}
