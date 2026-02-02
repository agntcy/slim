// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.examples.common;

import java.util.concurrent.CompletableFuture;

import io.agntcy.slim.bindings.App;
import io.agntcy.slim.bindings.ClientConfig;
import io.agntcy.slim.bindings.Name;
import io.agntcy.slim.bindings.Service;
import io.agntcy.slim.bindings.SlimBindings;

/**
 * Common utilities for SLIM Java binding examples.
 *
 * This class provides:
 * - Identity string parsing (org/namespace/app)
 * - App creation and connection helpers
 * - Default configuration values
 */
public class Common {

    /**
     * Default configuration values
     */
    public static final String DEFAULT_SERVER_ENDPOINT = "http://localhost:46357";
    public static final String DEFAULT_SHARED_SECRET = "demo-shared-secret-min-32-chars!!";

    /**
     * Splits an ID of form organization/namespace/application (or channel).
     *
     * @param id String in the canonical 'org/namespace/app-or-stream' format
     * @return Name object constructed from the ID
     * @throws IllegalArgumentException if the id cannot be split into exactly three
     *                                  segments
     */
    public static Name splitId(String id) {
        String[] parts = id.split("/");
        if (parts.length != 3) {
            throw new IllegalArgumentException(
                    "IDs must be in the format organization/namespace/app-or-stream, got: " + id);
        }
        return new Name(parts[0], parts[1], parts[2]);
    }

    /**
     * Result of creating and connecting an app.
     */
    public static class AppConnection {
        public final App app;
        public final Long connectionId;

        public AppConnection(App app, Long connectionId) {
            this.app = app;
            this.connectionId = connectionId;
        }
    }

    /**
     * Creates a SLIM app with shared secret authentication and connects it to a
     * SLIM server.
     *
     * This is a convenience function that combines:
     * - Crypto initialization
     * - App creation with shared secret
     * - Server connection with TLS settings
     *
     * @param localId    Local identity string (org/namespace/app format)
     * @param serverAddr SLIM server endpoint URL
     * @param secret     Shared secret for authentication (min 32 chars)
     * @return AppConnection containing the app and connection ID
     * @throws Exception if creation or connection fails
     */
    public static AppConnection createAndConnectApp(String localId, String serverAddr, String secret)
            throws Exception {
        // Initialize crypto, runtime, global service and logging with defaults
        SlimBindings.initializeWithDefaults();

        // Parse the local identity string
        Name appName = splitId(localId);

        // Create app with shared secret authentication
        Service globalService = SlimBindings.getGlobalService();
        App app = globalService.createAppWithSecret(appName, secret);

        // Connect to SLIM server (returns connection ID)
        ClientConfig config = SlimBindings.newInsecureClientConfig(serverAddr);
        Long connId = globalService.connectAsync(config).get();

        // Forward subscription to next node
        app.subscribe(app.name(), connId);

        return new AppConnection(app, connId);
    }

    /**
     * Creates a SLIM app with shared secret authentication and connects it to a
     * SLIM server (async version).
     *
     * @param localId    Local identity string (org/namespace/app format)
     * @param serverAddr SLIM server endpoint URL
     * @param secret     Shared secret for authentication (min 32 chars)
     * @return CompletableFuture of AppConnection
     */
    public static CompletableFuture<AppConnection> createAndConnectAppAsync(
            String localId, String serverAddr, String secret) {
        return CompletableFuture.supplyAsync(() -> {
            try {
                return createAndConnectApp(localId, serverAddr, secret);
            } catch (Exception e) {
                throw new RuntimeException(e);
            }
        });
    }
}
