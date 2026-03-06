// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings;

import org.junit.jupiter.api.Test;
import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.time.Duration;
import java.util.Map;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.Future;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.TimeoutException;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Integration tests for the slim_bindings Java layer.
 *
 * These tests exercise:
 * - End-to-end PointToPoint session creation, message publish/reply, and cleanup
 * - Session configuration retrieval and default session configuration propagation
 * - Automatic client reconnection after a server restart
 * - Error handling when targeting a non-existent subscription
 */
class BindingsTest {

    /**
     * Full round-trip:
     * - Two services connect (Alice, Bob)
     * - Subscribe & route setup
     * - PointToPoint session creation (Alice -> Bob)
     * - Publish + receive + reply
     * - Validate session IDs, payload integrity
     * - Test error behavior after deleting session
     * - Disconnect cleanup
     */
    @Test
    void testEndToEnd() throws Exception {
        TestHelpers.ServerFixture server = TestHelpers.setupServer("127.0.0.1:12344");

        try {
            System.out.println("[BindingsTest] testEndToEnd: starting");
            Name aliceName = new Name("org", "default", "alice_e2e");
            Name bobName = new Name("org", "default", "bob_e2e");

            Long connIdAlice = null;
            Long connIdBob = null;
            Service svcAlice;
            Service svcBob;

            if (server.localService()) {
                svcAlice = new Service("svcalice");
                svcBob = new Service("svcbob");
                connIdAlice = svcAlice.connect(server.getClientConfig());
                connIdBob = svcBob.connect(server.getClientConfig());
                System.out.println("[BindingsTest] testEndToEnd: connect");
            } else {
                svcAlice = server.service();
                svcBob = server.service();
            }

            App appAlice = svcAlice.createAppWithSecret(aliceName, TestHelpers.LONG_SECRET);
            App appBob = svcBob.createAppWithSecret(bobName, TestHelpers.LONG_SECRET);

            Name aliceNameFinal = aliceName;
            Name bobNameFinal = bobName;

            if (server.localService()) {
                aliceNameFinal = Name.newWithId("org", "default", "alice_e2e", appAlice.id());
                bobNameFinal = Name.newWithId("org", "default", "bob_e2e", appBob.id());
                appAlice.subscribe(aliceNameFinal, connIdAlice);
                appBob.subscribe(bobNameFinal, connIdBob);
                Thread.sleep(1000);
                appAlice.setRoute(bobNameFinal, connIdAlice);
                System.out.println("[BindingsTest] testEndToEnd: subscribe");
            }

            Thread.sleep(1000);

            SessionConfig sessionConfig = new SessionConfig(
                    SessionType.POINT_TO_POINT,
                    false,
                    5,
                    Duration.ofSeconds(1),
                    Map.of());

            Session sessionAlice = appAlice.createSessionAndWait(sessionConfig, bobNameFinal);
            System.out.println("[BindingsTest] testEndToEnd: session create");

            byte[] msg = new byte[]{1, 2, 3, 4, 5, 6, 7, 8, 9};
            sessionAlice.publishAndWait(msg, null, null);
            System.out.println("[BindingsTest] testEndToEnd: publish");

            Session sessionContextBob = appBob.listenForSession(Duration.ofSeconds(5));
            ReceivedMessage receivedMsg = sessionContextBob.getMessage(Duration.ofSeconds(5));
            System.out.println("[BindingsTest] testEndToEnd: receive");
            MessageContext messageCtx = receivedMsg.context();
            byte[] msgRcv = receivedMsg.payload();

            assertEquals(sessionContextBob.sessionId(), sessionAlice.sessionId());
            assertArrayEquals(msg, msgRcv);

            sessionContextBob.publishToAndWait(messageCtx, msgRcv, null, null);
            System.out.println("[BindingsTest] testEndToEnd: reply");

            ReceivedMessage receivedMsgAlice = sessionAlice.getMessage(Duration.ofSeconds(5));
            byte[] msgRcvAlice = receivedMsgAlice.payload();

            assertArrayEquals(msg, msgRcvAlice);

            appAlice.deleteSessionAndWait(sessionAlice);
            System.out.println("[BindingsTest] testEndToEnd: delete");

            if (connIdAlice != null) {
                svcAlice.disconnect(connIdAlice);
            }
        } finally {
            TestHelpers.teardownServer(server);
        }
    }

    /**
     * Test resilience / auto-reconnect:
     * - Establish connection and session
     * - Exchange a baseline message
     * - Stop and restart server
     * - Wait for automatic reconnection
     * - Publish again and confirm continuity using original session context
     */
    @Test
    void testAutoReconnectAfterServerRestart() throws Exception {
        String endpoint = "127.0.0.1:12346";

        System.out.println("[BindingsTest] testAutoReconnectAfterServerRestart: starting");
        Service svcServer = new Service("svcserver");
        Service svcAlice = new Service("svcalice");
        Service svcBob = new Service("svcbob");

        ServerConfig serverConf = SlimBindings.newInsecureServerConfig(endpoint);
        svcServer.runServerAsync(serverConf).get();

        ClientConfig clientConf = SlimBindings.newInsecureClientConfig("http://" + endpoint);
        Long connIdAlice = svcAlice.connect(clientConf);
        Long connIdBob = svcBob.connect(clientConf);
        System.out.println("[BindingsTest] testAutoReconnectAfterServerRestart: connect");

        Name aliceName = new Name("org", "default", "alice_res");
        Name bobName = new Name("org", "default", "bob_res");

        App appAlice = svcAlice.createAppWithSecret(aliceName, TestHelpers.LONG_SECRET);
        App appBob = svcBob.createAppWithSecret(bobName, TestHelpers.LONG_SECRET);

        Name aliceNameWithId = Name.newWithId("org", "default", "alice_res", appAlice.id());
        Name bobNameWithId = Name.newWithId("org", "default", "bob_res", appBob.id());

        appAlice.subscribe(aliceNameWithId, connIdAlice);
        appBob.subscribe(bobNameWithId, connIdBob);
        Thread.sleep(1000);

        appAlice.setRoute(bobNameWithId, connIdAlice);
        Thread.sleep(1000);

        SessionConfig sessionConfig = new SessionConfig(
                SessionType.POINT_TO_POINT,
                false,
                5,
                Duration.ofSeconds(1),
                Map.of());

        Session sessionAlice = appAlice.createSessionAndWait(sessionConfig, bobNameWithId);
        System.out.println("[BindingsTest] testAutoReconnectAfterServerRestart: session create");

        byte[] baselineMsg = new byte[]{1, 2, 3};
        sessionAlice.publishAndWait(baselineMsg, null, null);

        Session bobSessionCtx = appBob.listenForSession(Duration.ofSeconds(5));
        ReceivedMessage receivedMsg = bobSessionCtx.getMessage(Duration.ofSeconds(5));
        byte[] received = receivedMsg.payload();
        assertArrayEquals(baselineMsg, received);

        assertEquals(bobSessionCtx.sessionId(), sessionAlice.sessionId());
        System.out.println("[BindingsTest] testAutoReconnectAfterServerRestart: baseline message ok");

        svcServer.shutdownAsync().get();
        Thread.sleep(3000);
        System.out.println("[BindingsTest] testAutoReconnectAfterServerRestart: server stopped");

        Service svcServerNew = new Service("svcserver");
        svcServerNew.runServerAsync(SlimBindings.newInsecureServerConfig(endpoint)).get();
        Thread.sleep(10000);
        System.out.println("[BindingsTest] testAutoReconnectAfterServerRestart: server restarted, reconnecting");

        byte[] testMsg = new byte[]{4, 5, 6};
        sessionAlice.publishAndWait(testMsg, null, null);

        ReceivedMessage receivedMsg2 = bobSessionCtx.getMessage(Duration.ofSeconds(5));
        byte[] received2 = receivedMsg2.payload();
        assertArrayEquals(testMsg, received2);

        appAlice.deleteSessionAndWait(sessionAlice);

        svcAlice.disconnect(connIdAlice);
        svcBob.disconnect(connIdBob);
        svcServerNew.shutdownAsync().get();
    }

    /**
     * Validate error path when publishing to an unsubscribed / nonexistent destination:
     * - Create only Alice, subscribe her
     * - Publish message addressed to Bob (not connected)
     * - Expect an error surfaced (no matching subscription)
     */
    @Test
    void testErrorOnNonexistentSubscription() throws Exception {
        TestHelpers.ServerFixture server = TestHelpers.setupServer("127.0.0.1:12347");

        try {
            Name aliceName = new Name("org", "default", "alice_nonsub");

            Long connIdAlice = null;
            Service svcAlice;
            App appAlice;

            if (server.localService()) {
                svcAlice = new Service("svcalice");
                connIdAlice = svcAlice.connect(server.getClientConfig());
                appAlice = svcAlice.createAppWithSecret(aliceName, TestHelpers.LONG_SECRET);
                Name aliceNameWithId = Name.newWithId("org", "default", "alice_nonsub", appAlice.id());
                appAlice.subscribe(aliceNameWithId, connIdAlice);
            } else {
                svcAlice = server.service();
                appAlice = svcAlice.createAppWithSecret(aliceName, TestHelpers.LONG_SECRET);
            }

            Name bobName = new Name("org", "default", "bob_nonsub");

            SessionConfig sessionConfig = new SessionConfig(
                    SessionType.POINT_TO_POINT,
                    false,
                    null,
                    null,
                    Map.of());

            assertThrows(Exception.class, () ->
                    appAlice.createSessionAndWait(sessionConfig, bobName));

            if (connIdAlice != null) {
                svcAlice.disconnect(connIdAlice);
            }
        } finally {
            TestHelpers.teardownServer(server);
        }
    }

    /**
     * Test that listen_for_session times out appropriately when no session is available.
     */
    @ParameterizedTest
    @ValueSource(strings = {"127.0.0.1:12348", ""})
    void testListenForSessionTimeout(String endpoint) throws Exception {
        String ep = endpoint.isEmpty() ? null : endpoint;
        TestHelpers.ServerFixture server = TestHelpers.setupServer(ep);

        try {
            System.out.println("[BindingsTest] testListenForSessionTimeout: endpoint=" + ep);
            Name aliceName = new Name("org", "default", "alice_timeout");

            Long connIdAlice = null;
            Service svcAlice;
            App appAlice;

            if (server.localService()) {
                svcAlice = new Service("svcalice");
                connIdAlice = svcAlice.connect(server.getClientConfig());
                appAlice = svcAlice.createAppWithSecret(aliceName, TestHelpers.LONG_SECRET);
                Name aliceNameWithId = Name.newWithId("org", "default", "alice_timeout", appAlice.id());
                appAlice.subscribe(aliceNameWithId, connIdAlice);
            } else {
                svcAlice = server.service();
                appAlice = svcAlice.createAppWithSecret(aliceName, TestHelpers.LONG_SECRET);
            }

            long startTime = System.currentTimeMillis();
            Duration timeout = Duration.ofMillis(100);

            Exception exception = assertThrows(Exception.class, () ->
                    appAlice.listenForSession(timeout));

            double elapsedTime = (System.currentTimeMillis() - startTime) / 1000.0;
            System.out.println("[BindingsTest] testListenForSessionTimeout: timeout asserted elapsed=" + elapsedTime + "s");

            assertTrue(elapsedTime >= 0.08 && elapsedTime <= 0.2,
                    "Timeout took " + elapsedTime + "s, expected ~0.1s");
            assertTrue(
                    (exception.getMessage() != null && exception.getMessage().toLowerCase().contains("timed out"))
                            || (exception.getCause() != null
                                    && exception.getCause().getMessage() != null
                                    && exception.getCause().getMessage().toLowerCase().contains("timeout")));

            ExecutorService executor = Executors.newSingleThreadExecutor();
            try {
                Future<?> future = executor.submit(() -> {
                    try {
                        appAlice.listenForSession(null);
                    } catch (Exception e) {
                        throw new RuntimeException(e);
                    }
                });
                assertThrows(TimeoutException.class, () -> future.get(100, TimeUnit.MILLISECONDS));
            } finally {
                executor.shutdownNow();
            }

            if (connIdAlice != null) {
                svcAlice.disconnect(connIdAlice);
            }
        } finally {
            TestHelpers.teardownServer(server);
        }
    }
}
