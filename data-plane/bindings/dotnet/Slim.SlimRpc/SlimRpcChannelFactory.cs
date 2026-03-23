// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

using Agntcy.Slim;

namespace Agntcy.Slim.SlimRpc;

/// <summary>
/// Factory for creating slimrpc channels from SlimApp.
/// </summary>
public static class SlimRpcChannelFactory
{
    /// <summary>
    /// Create an RPC channel to the remote service.
    /// </summary>
    /// <param name="app">The Slim application.</param>
    /// <param name="remote">The remote service name.</param>
    /// <param name="connectionId">Optional connection ID to use.</param>
    /// <returns>A channel for making RPC calls.</returns>
    public static uniffi.slim_bindings.Channel CreateChannel(
        SlimApp app,
        SlimName remote,
        ulong? connectionId = null)
    {
        ArgumentNullException.ThrowIfNull(app);
        ArgumentNullException.ThrowIfNull(remote);
        return uniffi.slim_bindings.Channel.NewWithConnection(app._inner, remote._inner, connectionId);
    }
}
