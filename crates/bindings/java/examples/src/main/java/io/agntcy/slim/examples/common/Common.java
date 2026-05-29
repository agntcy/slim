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
    public static final String DEFAULT_SHARED_SECRET = "my_shared_secret_for_testing_purposes_only";

    /**
     * Returns the SLIM server endpoint.
     * Checks the SLIM_ADDR environment variable first, falling back to DEFAULT_SERVER_ENDPOINT.
     */
    public static String getServerEndpoint() {
        String env = System.getenv("SLIM_ADDR");
        return (env != null && !env.isEmpty()) ? env : DEFAULT_SERVER_ENDPOINT;
    }

    /** Name org component used across all examples. */
    public static final String NAME_ORG = "agntcy";

    /** Name namespace component used across all examples. */
    public static final String NAME_NS = "grpc";

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
        Name appName = Name.fromString(localId);

        // Create app with shared secret authentication
        Service globalService = SlimBindings.getGlobalService();
        App app = globalService.createAppWithSecret(appName, secret);

        // Connect to SLIM server (returns connection ID)
        ClientConfig config = SlimBindings.newInsecureClientConfig(serverAddr);
        Long connId = globalService.connect(config);

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
