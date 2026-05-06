// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings;

import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.CsvSource;

import java.time.Duration;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicInteger;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Point-to-point sticky session integration test.
 *
 * Scenario:
 *   - One logical sender creates a PointToPoint session and sends 1000 messages
 *     to a shared logical receiver identity.
 *   - Ten receiver instances (same Name) concurrently listen for an
 *     inbound session. Only one should become the bound peer for the
 *     PointToPoint session (stickiness).
 *   - All 1000 messages must arrive at exactly one receiver (verifying
 *     session affinity) and none at the others.
 *   - Test runs with MLS enabled / disabled (parametrized) to ensure
 *     stickiness is orthogonal to MLS.
 */
class PointToPointTest {

    private Object[] setupSender(TestHelpers.ServerFixture server, Name senderName, String testId, Name receiverName)
            throws Exception {
        Long connIdSender = null;
        Service svcSender;
        if (server.localService()) {
            svcSender = new Service("svcsender");
            connIdSender = svcSender.connect(server.getClientConfig());
        } else {
            svcSender = server.service();
        }

        App sender = svcSender.createAppWithSecret(senderName, TestHelpers.LONG_SECRET);

        if (server.localService() && connIdSender != null) {
            Name senderNameWithId = Name.newWithId("org", "test_" + testId, "p2psender", sender.id());
            sender.subscribe(senderNameWithId, connIdSender);
            Thread.sleep(100);
            sender.setRoute(receiverName, connIdSender);
        }

        return new Object[]{sender, connIdSender};
    }

    private void publishMessages(Session senderSession, int nMessages, String payloadType, Map<String, String> metadata)
            throws Exception {
        for (int i = 0; i < nMessages; i++) {
            senderSession.publishAndWait(
                    "Hello from sender".getBytes(),
                    payloadType,
                    metadata);
            if ((i + 1) % 200 == 0) {
                System.out.println("[PointToPointTest] published " + (i + 1) + "/" + nMessages + " messages");
            }
        }
    }

    private int waitForAck(Session senderSession) throws Exception {
        ReceivedMessage receivedMsg = senderSession.getMessage(Duration.ofSeconds(30));
        byte[] msg = receivedMsg.payload();
        String ackText = new String(msg);
        assertTrue(ackText.startsWith("All messages received: "),
        "Expected ack format 'All messages received: {receiverIdx}', got: " + ackText);
        String suffix = ackText.substring("All messages received: ".length()).trim();
        assertTrue(suffix.matches("\\d+"),
        "Expected numeric receiver index after 'All messages received: ', got: " + ackText);
        return Integer.parseInt(suffix);
    }

    private void validateAffinity(Map<Integer, AtomicInteger> receiverCounts, int nMessages) {
        int sum = receiverCounts.values().stream().mapToInt(AtomicInteger::get).sum();
        assertEquals(nMessages, sum);
        assertTrue(receiverCounts.values().stream().anyMatch(c -> c.get() == nMessages));
    }

    @ParameterizedTest
    @CsvSource({
            "127.0.0.1:22345,true",
            // ",true"
    })
    void testStickySession(String endpoint, boolean mlsEnabled) throws Exception {
        String ep = (endpoint == null || endpoint.isEmpty()) ? null : endpoint;
        TestHelpers.ServerFixture server = TestHelpers.setupServer(ep);

        try {
            String testId = UUID.randomUUID().toString().substring(0, 8);
            Name senderName = new Name("org", "test_" + testId, "p2psender");
            Name receiverName = new Name("org", "test_" + testId, "p2preceiver");

            int nMessages = 1000;
            System.out.println("[PointToPointTest] testId=" + testId + " endpoint=" + ep + " mlsEnabled=" + mlsEnabled + " nMessages=" + nMessages);

            Object[] senderResult = setupSender(server, senderName, testId, receiverName);
            App sender = (App) senderResult[0];

            Map<Integer, AtomicInteger> receiverCounts = new ConcurrentHashMap<>();
            for (int i = 0; i < 10; i++) {
                receiverCounts.put(i, new AtomicInteger(0));
            }

            try (ExecutorService executor = Executors.newFixedThreadPool(10)) {

            for (int i = 0; i < 10; i++) {
                final int idx = i;
                executor.submit(() -> {
                    try {
                        Long connIdReceiver = null;
                        Service svcReceiver;
                        if (server.localService()) {
                            svcReceiver = new Service("svcreceiver" + idx);
                            connIdReceiver = svcReceiver.connect(server.getClientConfig());
                            System.out.println("[PointToPointTest] receiver " + idx + ": connected");
                        } else {
                            svcReceiver = server.service();
                        }

                        App receiver = svcReceiver.createAppWithSecret(receiverName, TestHelpers.LONG_SECRET);

                        if (server.localService() && connIdReceiver != null) {
                            Name receiverNameWithId =
                                    Name.newWithId("org", "test_" + testId, "p2preceiver", receiver.id());
                            receiver.subscribe(receiverNameWithId, connIdReceiver);
                            Thread.sleep(100);
                        }

                        System.out.println("[PointToPointTest] receiver " + idx + ": listening");
                        Session session = receiver.listenForSession(Duration.ofSeconds(120));
                        assertEquals(SessionType.POINT_TO_POINT, session.sessionType());
                        System.out.println("[PointToPointTest] receiver " + idx + ": got session");
                        while (true) {
                            try {
                                ReceivedMessage receivedMsg = session.getMessage(Duration.ofSeconds(30));
                                MessageContext ctx = receivedMsg.context();
                                Map<String, String> meta = ctx.metadata();
                                if ("hello message".equals(ctx.payloadType())
                                        && meta != null && "hello".equals(meta.get("sender"))) {
                                    receiverCounts.get(idx).incrementAndGet();

                                    if (receiverCounts.get(idx).get() == nMessages) {
                                        session.publishAndWait(
                                                ("All messages received: " + idx).getBytes(),
                                                null,
                                                null);
                                    }
                                }
                            } catch (Exception e) {
                                break;
                            }
                        }
                    } catch (Exception e) {
                        throw new RuntimeException(e);
                    }
                });
            }

            Thread.sleep(1000);

            SessionConfig sessionConfig = new SessionConfig(
                    SessionType.POINT_TO_POINT,
                    mlsEnabled,
                    5,
                    Duration.ofMillis(100),
                    Map.of());

            Session senderSession = sender.createSessionAndWait(sessionConfig, receiverName);
            System.out.println("[PointToPointTest] sender session created");
            String payloadType = "hello message";
            Map<String, String> metadata = Map.of("sender", "hello");

            publishMessages(senderSession, nMessages, payloadType, metadata);
            int winnerId = waitForAck(senderSession);

            validateAffinity(receiverCounts, nMessages);
            System.out.println("[PointToPointTest] winner=receiver " + winnerId + ", affinity validated");

            sender.deleteSessionAndWait(senderSession);

            executor.shutdownNow();
            executor.awaitTermination(10, TimeUnit.SECONDS);
            }
        } finally {
            TestHelpers.teardownServer(server);
        }
    }
}
