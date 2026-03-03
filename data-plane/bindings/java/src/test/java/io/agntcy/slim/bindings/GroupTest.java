// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package io.agntcy.slim.bindings;

import org.junit.jupiter.params.ParameterizedTest;
import org.junit.jupiter.params.provider.ValueSource;

import java.time.Duration;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.*;
import java.util.concurrent.atomic.AtomicInteger;

import static org.junit.jupiter.api.Assertions.*;

/**
 * Group integration test for Slim bindings.
 *
 * Scenario:
 *   - A configurable number of participants join a group "chat" identified by a shared topic (Name).
 *   - Participant 0 (the "moderator") creates the group session, invites every other participant,
 *     and publishes the first message addressed to the next participant in a logical ring.
 *   - Each participant, upon receiving a message that ends with its own name, publishes a new
 *     message naming the next participant, continuing the ring.
 *   - Each participant exits after observing (participants_count - 1) messages.
 */
class GroupTest {

    private Session createGroupSession(App participant, Name chatName, boolean mlsEnabled) throws Exception {
        SessionConfig sessionConfig = new SessionConfig(
                SessionType.GROUP,
                mlsEnabled,
                5,
                Duration.ofSeconds(1),
                Map.of());
        return participant.createSessionAndWait(sessionConfig, chatName);
    }

    private void inviteParticipants(App participant, Session session, TestHelpers.ServerFixture server,
            String testId, int participantsCount, Long connId, String partName) throws Exception {
        for (int i = 0; i < participantsCount; i++) {
            if (i != 0) {
                String nameToAdd = "participant_" + i;
                Name toAdd = new Name("org", "test_" + testId, nameToAdd);
                if (server.localService() && connId != null) {
                    participant.setRoute(toAdd, connId);
                }
                session.inviteAndWait(toAdd);
            }
        }
        Thread.sleep(1000);
    }

    @ParameterizedTest
    @ValueSource(booleans = {false})
    void testGroup(boolean mlsEnabled) throws Exception {
        TestHelpers.ServerFixture server = TestHelpers.setupServer(null);

        try {
            String message = "Calling app";
            String testId = UUID.randomUUID().toString().substring(0, 8);
            int participantsCount = 10;

            System.out.println("[GroupTest] testId=" + testId + " mlsEnabled=" + mlsEnabled + " participants=" + participantsCount);

            CountDownLatch allReady = new CountDownLatch(1);
            AtomicInteger readyCount = new AtomicInteger(0);

            Name chatName = new Name("org", "test_" + testId, "chat");

            ExecutorService executor = Executors.newFixedThreadPool(participantsCount);

            for (int index = 0; index < participantsCount; index++) {
                final int idx = index;
                executor.submit(() -> {
                    try {
                        TestHelpers.ParticipantResult pr = TestHelpers.createParticipant(server, testId, idx);
                        App participant = pr.app;
                        Long connId = pr.connectionId;
                        String partName = pr.participantName;

                        System.out.println("[GroupTest] participant " + idx + ": created (" + partName + ")");

                        Session session;
                        if (idx == 0) {
                            session = createGroupSession(participant, chatName, mlsEnabled);
                            System.out.println("[GroupTest] moderator: session created");
                            allReady.await(30, TimeUnit.SECONDS);
                            System.out.println("[GroupTest] moderator: inviting " + (participantsCount - 1) + " participants");
                            inviteParticipants(participant, session, server, testId, participantsCount, connId, partName);
                            System.out.println("[GroupTest] moderator: sent first message to participant_1");
                        } else {
                            System.out.println("[GroupTest] participant " + idx + ": listening for session");
                            if (readyCount.incrementAndGet() == participantsCount - 1) {
                                allReady.countDown();
                            }
                            session = participant.listenForSession(null);
                            assertEquals(SessionType.GROUP, session.sessionType());
                            System.out.println("[GroupTest] participant " + idx + ": got session");
                        }

                        int localCount = 0;
                        boolean called = false;

                        while (localCount < participantsCount - 1) {
                            if (idx == 0 && !called) {
                                int nextParticipant = (idx + 1) % participantsCount;
                                String nextParticipantName = "participant_" + nextParticipant;
                                String msg = message + " - " + nextParticipantName;
                                called = true;
                                session.publishAndWait(msg.getBytes(), null, null);
                            }

                            ReceivedMessage receivedMsg = session.getMessage(Duration.ofSeconds(30));
                            byte[] msgRcv = receivedMsg.payload();
                            localCount++;

                            System.out.println("[GroupTest] participant " + idx + ": received msg localCount=" + localCount);

                            assertTrue(new String(msgRcv).startsWith(message));

                            String msgStr = new String(msgRcv);
                            if (!called && msgStr.endsWith(partName)) {
                                int nextParticipant = (idx + 1) % participantsCount;
                                String nextParticipantName = "participant_" + nextParticipant;
                                session.publishAndWait((message + " - " + nextParticipantName).getBytes(), null, null);
                                called = true;
                                System.out.println("[GroupTest] participant " + idx + ": publishing to " + nextParticipantName);
                            }

                            if (localCount >= participantsCount - 1) {
                                System.out.println("[GroupTest] participant " + idx + ": done (localCount=" + localCount + ")");
                                if (idx == 0) {
                                    Thread.sleep(500);
                                    participant.deleteSessionAndWait(session);
                                    System.out.println("[GroupTest] moderator: deleted session");
                                }
                                break;
                            }
                        }
                    } catch (Exception e) {
                        throw new RuntimeException(e);
                    }
                });
            }

            executor.shutdown();
            assertTrue(executor.awaitTermination(180, TimeUnit.SECONDS));
            System.out.println("[GroupTest] all participants completed");
        } finally {
            TestHelpers.teardownServer(server);
        }
    }
}
