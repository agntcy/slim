// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings;

import java.util.List;

/**
 * Shared constants and helpers for Java bindings tests.
 */
public final class TestHelpers {

    /** Shared secret for tests (min 32 chars). */
    public static final String LONG_SECRET = "e4aaecb9ae0b23b82086bb8a8633e01fba16ae8d9c1379a613c00838";

    private TestHelpers() {}

    /**
     * Server fixture for managing local and global service instances.
     */
    public static final class ServerFixture {
        private final Service service;
        private final String endpoint;

        public ServerFixture(Service service, String endpoint) {
            this.service = service;
            this.endpoint = endpoint;
        }

        public Service service() {
            return service;
        }

        public String endpoint() {
            return endpoint;
        }

        public boolean localService() {
            return endpoint != null;
        }

        public ClientConfig getClientConfig() {
            return localService() ? SlimBindings.newInsecureClientConfig("http://" + endpoint) : null;
        }
    }

    /**
     * Setup a server fixture with the given endpoint.
     *
     * @param endpoint Endpoint string (e.g., "127.0.0.1:12345") or null for global service
     * @return ServerFixture instance
     */
    public static ServerFixture setupServer(String endpoint) throws Exception {
        TracingConfig tracingConfig = SlimBindings.newTracingConfig();
        RuntimeConfig runtimeConfig = SlimBindings.newRuntimeConfig();
        ServiceConfig serviceConfig = SlimBindings.newServiceConfig();

        tracingConfig.setLogLevel("info");
        SlimBindings.initializeWithConfigs(runtimeConfig, tracingConfig, List.of(serviceConfig));

        Service service;
        if (endpoint != null) {
            Service svc = new Service("localserver");
            ServerConfig serverConfig = SlimBindings.newInsecureServerConfig(endpoint);
            svc.runServerAsync(serverConfig).get();
            service = svc;
        } else {
            service = SlimBindings.getGlobalService();
        }

        Thread.sleep(1000);
        return new ServerFixture(service, endpoint);
    }

    /**
     * Teardown the server fixture.
     */
    public static void teardownServer(ServerFixture fixture) {
        if (fixture.endpoint() != null) {
            try {
                fixture.service().shutdown();
            } catch (Exception e) {
                System.out.println("Warning: error stopping server " + fixture.endpoint() + ": " + e.getMessage());
            }
        }
    }

    /**
     * Result of creating a participant.
     */
    public static final class ParticipantResult {
        public final App app;
        public final Long connectionId;
        public final String participantName;

        public ParticipantResult(App app, Long connectionId, String participantName) {
            this.app = app;
            this.connectionId = connectionId;
            this.participantName = participantName;
        }
    }

    /**
     * Create a participant app with unique naming.
     */
    public static ParticipantResult createParticipant(ServerFixture server, String testId, int index)
            throws Exception {
        String partName = "participant_" + index;
        Name name = new Name("org", "test_" + testId, partName);

        Long connId;
        Service svc;
        if (server.localService()) {
            svc = new Service("svcparticipant" + index);
            ClientConfig clientConfig = server.getClientConfig();
            if (clientConfig == null) {
                throw new IllegalStateException("Client config should not be null for local service");
            }
            connId = svc.connect(clientConfig);
        } else {
            svc = server.service();
            connId = null;
        }

        App participant = svc.createAppWithSecret(name, LONG_SECRET);

        if (server.localService() && connId != null) {
            Name nameWithId = Name.newWithId("org", "test_" + testId, partName, participant.id());
            participant.subscribe(nameWithId, connId);
            Thread.sleep(100);
        }

        return new ParticipantResult(participant, connId, partName);
    }
}
