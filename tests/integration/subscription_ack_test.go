// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

// Subscription ACK compatibility tests
//
// These tests verify that the subscription ACK mechanism works correctly when
// mixing old (pre-1.2.0) and new (≥1.2.0) relay nodes and applications.
//
// Every test that exercises subscription behaviour also verifies end-to-end
// message delivery: after subscriptions are established, app B sends "hey" to
// app A; app A replies "hello from the a", and app B must receive that reply.
//
// Scenarios:
//   - new app ↔ new relay: remote ACK path.
//   - new app ↔ old relay: default path.
//   - old relay as upstream of new relay: mixed paths.
//   - Legacy sdk-mock (pre-1.2.0) interoperability with old and new relays.

import (
	"fmt"
	"os"
	"os/exec"
	"time"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	"github.com/onsi/gomega/gbytes"
	"github.com/onsi/gomega/gexec"
)

var _ = Describe("Subscription ACK Compatibility", func() {
	var tempDir string

	BeforeEach(func() {
		tempDir = newTempDir("slim-integration-sub-ack-")
	})

	AfterEach(func() {
		if tempDir != "" {
			_ = os.RemoveAll(tempDir)
			tempDir = ""
		}
	})

	// ── new app ↔ new relay ──────────────────────────────────────────────────
	//
	// Both nodes run ≥ 1.2.0.  The app's embedded relay uses the remote ACK
	// path.  After the subscription round-trip completes, a second app sends a
	// message to confirm end-to-end routing.
	Describe("new relay server with new app", func() {
		It("completes subscription via remote ack path and delivers messages", func() {
			relayPort := reservePort()

			replacements := map[string]string{
				"0.0.0.0:46490":         fmt.Sprintf("0.0.0.0:%d", relayPort),
				"http://localhost:46490": fmt.Sprintf("http://localhost:%d", relayPort),
			}

			relayConfig := writeTempConfig(tempDir, "./testdata/sub-ack-relay-config.yaml", "relay.yaml", replacements)

			relaySession, err := gexec.Start(
				exec.Command(slimPath, "--config", relayConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(relaySession, 5*time.Second)

			Eventually(relaySession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			// App A: subscriber.
			appAConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "app-a.yaml", replacements)
			appASession, err := gexec.Start(
				exec.Command(sdkMockPath,
					"--config", appAConfig,
					"--local-name", "a",
					"--remote-name", "b",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appASession, 5*time.Second)

			// Link negotiation must complete so the app's embedded relay learns
			// the upstream relay's version.
			Eventually(relaySession.Out, 10*time.Second).Should(gbytes.Say("received link negotiation"))
			Eventually(relaySession.Out, 10*time.Second).Should(gbytes.Say("1.2.0"))

			// App's embedded relay must choose the remote ack path.
			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("subscription: remote ack path"))

			// The external relay (no upstream) takes the default path and sends
			// a SubscriptionAck back.
			Eventually(relaySession.Out, 10*time.Second).Should(gbytes.Say("subscription: default ack path"))

			// App's embedded relay receives the SubscriptionAck: subscription is
			// fully confirmed before we start the sender.
			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("subscription: remote ack received"))

			// App B: sender.
			appBConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "app-b.yaml", replacements)
			appBSession, err := gexec.Start(
				exec.Command(sdkMockPath,
					"--config", appBConfig,
					"--local-name", "b",
					"--remote-name", "a",
					"--message", "hey",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appBSession, 5*time.Second)

			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("Queueing reply"))
			Eventually(appBSession.Out, 10*time.Second).Should(gbytes.Say("hello from the a"))
		})
	})

	// ── new app ↔ old relay ──────────────────────────────────────────────────
	//
	// The old relay never replies to link negotiation, so the app's embedded
	// relay falls back to the default path.  Message delivery must still work.
	Describe("old relay server with new app", func() {
		It("subscribes via the default path and delivers messages", func() {
			relayPort := reservePort()

			replacements := map[string]string{
				"0.0.0.0:46490":         fmt.Sprintf("0.0.0.0:%d", relayPort),
				"http://localhost:46490": fmt.Sprintf("http://localhost:%d", relayPort),
			}

			// Old relay as server.
			relayConfig := writeTempConfig(tempDir, "./testdata/sub-ack-relay-config.yaml", "legacy-relay.yaml", replacements)
			legacyRelaySession, err := gexec.Start(
				exec.Command(legacySlimPath, "--config", relayConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(legacyRelaySession, 5*time.Second)

			Eventually(legacyRelaySession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			// App A: subscriber, connects to old relay.
			appAConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "app-a.yaml", replacements)
			appASession, err := gexec.Start(
				exec.Command(sdkMockPath,
					"--config", appAConfig,
					"--local-name", "a",
					"--remote-name", "b",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appASession, 5*time.Second)

			// App A's embedded relay must fall back to the default path because
			// the old relay never responds to link negotiation (supports()=false).
			// The ACK is immediate so the subscription is ready as soon as this
			// log line appears.
			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("subscription: default ack path"))

			// App B: sender.
			appBConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "app-b.yaml", replacements)
			appBSession, err := gexec.Start(
				exec.Command(sdkMockPath,
					"--config", appBConfig,
					"--local-name", "b",
					"--remote-name", "a",
					"--message", "hey",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appBSession, 5*time.Second)

			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("Queueing reply"))
			Eventually(appBSession.Out, 10*time.Second).Should(gbytes.Say("hello from the a"))
		})
	})

	// ── old relay as upstream of new relay ───────────────────────────────────
	//
	// The new relay detects the old relay via link negotiation and uses the
	// default path toward it.  A new app on the new relay uses the remote-ack
	// path.  End-to-end message delivery is verified alongside the ACK checks.
	Describe("old relay as client of new relay, new app on new relay", func() {
		It("uses correct ack paths and delivers messages", func() {
			newRelayPort := reservePort()

			replacements := map[string]string{
				"0.0.0.0:46490":         fmt.Sprintf("0.0.0.0:%d", newRelayPort),
				"http://localhost:46490": fmt.Sprintf("http://localhost:%d", newRelayPort),
			}

			// New relay as server.
			newRelayConfig := writeTempConfig(tempDir, "./testdata/sub-ack-relay-config.yaml", "new-relay.yaml", replacements)
			newRelaySession, err := gexec.Start(
				exec.Command(slimPath, "--config", newRelayConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(newRelaySession, 5*time.Second)

			Eventually(newRelaySession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			// Old relay connects to new relay as a client (relay-to-relay link).
			legacyClientConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "legacy-client.yaml", replacements)
			legacyClientSession, err := gexec.Start(
				exec.Command(legacySlimPath, "--config", legacyClientConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(legacyClientSession, 5*time.Second)

			Eventually(newRelaySession.Out, 10*time.Second).Should(gbytes.Say("new connection received from remote"))

			// App A: subscriber on the new relay.
			appAConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "app-a.yaml", replacements)
			appASession, err := gexec.Start(
				exec.Command(sdkMockPath,
					"--config", appAConfig,
					"--local-name", "a",
					"--remote-name", "b",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appASession, 5*time.Second)

			// Link negotiation between the new relay and new app confirms
			// remote-ack path is available.
			Eventually(newRelaySession.Out, 10*time.Second).Should(gbytes.Say("received link negotiation"))
			Eventually(newRelaySession.Out, 10*time.Second).Should(gbytes.Say("1.2.0"))

			// App A's embedded relay uses the remote ack path toward the new relay.
			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("subscription: remote ack path"))

			// New relay processes the subscription with default path (no further
			// upstream for this subscription).
			Eventually(newRelaySession.Out, 10*time.Second).Should(gbytes.Say("subscription: default ack path"))

			// App A's subscription is fully confirmed.
			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("subscription: remote ack received"))

			// App B: sender, also on the new relay.
			appBConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "app-b.yaml", replacements)
			appBSession, err := gexec.Start(
				exec.Command(sdkMockPath,
					"--config", appBConfig,
					"--local-name", "b",
					"--remote-name", "a",
					"--message", "hey",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appBSession, 5*time.Second)

			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("Queueing reply"))
			Eventually(appBSession.Out, 10*time.Second).Should(gbytes.Say("hello from the a"))

			// Old relay must keep running throughout.
			Consistently(legacyClientSession, 500*time.Millisecond).ShouldNot(gexec.Exit())
		})
	})

	// ── message delivery: two new apps through new relay (remote ack) ────────
	//
	// Two new sdk-mock instances connect to a single new relay.  Each registers
	// its subscription via the remote-ack path.  App B sends a message to app A.
	Describe("message delivery through new relay (remote ack path)", func() {
		It("routes messages between two new apps via the new relay", func() {
			relayPort := reservePort()

			replacements := map[string]string{
				"0.0.0.0:46490":         fmt.Sprintf("0.0.0.0:%d", relayPort),
				"http://localhost:46490": fmt.Sprintf("http://localhost:%d", relayPort),
			}

			relayConfig := writeTempConfig(tempDir, "./testdata/sub-ack-relay-config.yaml", "relay.yaml", replacements)
			relaySession, err := gexec.Start(
				exec.Command(slimPath, "--config", relayConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(relaySession, 5*time.Second)

			Eventually(relaySession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			// App A: subscriber.
			appAConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "app-a.yaml", replacements)
			appASession, err := gexec.Start(
				exec.Command(sdkMockPath,
					"--config", appAConfig,
					"--local-name", "a",
					"--remote-name", "b",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appASession, 5*time.Second)

			// Gate on the remote-ack round-trip completing before B starts sending.
			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("subscription: remote ack received"))

			// App B: sender.
			appBConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "app-b.yaml", replacements)
			appBSession, err := gexec.Start(
				exec.Command(sdkMockPath,
					"--config", appBConfig,
					"--local-name", "b",
					"--remote-name", "a",
					"--message", "hey",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appBSession, 5*time.Second)

			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("Queueing reply"))
			Eventually(appBSession.Out, 10*time.Second).Should(gbytes.Say("hello from the a"))
		})
	})

	// ── message delivery: two new apps through old relay (default path) ──────
	//
	// Same two-app scenario but the central relay is the legacy binary.  The
	// new apps' embedded relays detect the old relay and fall back to the
	// default path.  Routing must still work end-to-end.
	Describe("message delivery through old relay (default path)", func() {
		It("routes messages between two new apps via the old relay", func() {
			relayPort := reservePort()

			replacements := map[string]string{
				"0.0.0.0:46490":         fmt.Sprintf("0.0.0.0:%d", relayPort),
				"http://localhost:46490": fmt.Sprintf("http://localhost:%d", relayPort),
			}

			// Old relay as central hub.
			relayConfig := writeTempConfig(tempDir, "./testdata/sub-ack-relay-config.yaml", "legacy-relay.yaml", replacements)
			relaySession, err := gexec.Start(
				exec.Command(legacySlimPath, "--config", relayConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(relaySession, 5*time.Second)

			Eventually(relaySession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			// App A: subscriber.
			appAConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "app-a.yaml", replacements)
			appASession, err := gexec.Start(
				exec.Command(sdkMockPath,
					"--config", appAConfig,
					"--local-name", "a",
					"--remote-name", "b",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appASession, 5*time.Second)

			// App A's embedded relay falls back to the default path.  The ACK is
			// immediate; a brief pause lets the subscribe propagate through the
			// old relay's internal state before B starts sending.
			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("subscription: default ack path"))
			time.Sleep(500 * time.Millisecond)

			// App B: sender.
			appBConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "app-b.yaml", replacements)
			appBSession, err := gexec.Start(
				exec.Command(sdkMockPath,
					"--config", appBConfig,
					"--local-name", "b",
					"--remote-name", "a",
					"--message", "hey",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appBSession, 5*time.Second)

			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("Queueing reply"))
			Eventually(appBSession.Out, 10*time.Second).Should(gbytes.Say("hello from the a"))
		})
	})

	// ── legacy sdk-mock scenarios ─────────────────────────────────────────────
	//
	// These tests verify that an old application (pre-1.2.0 sdk-mock binary)
	// interoperates correctly with both old and new relays.  All tests are
	// skipped when the legacy sdk-mock binary is not present in .dist/bin/.

	// ── old app ↔ new relay ───────────────────────────────────────────────────
	//
	// The old sdk-mock's embedded relay never performs link negotiation, so the
	// new relay sees no version for that connection.  When the new relay
	// receives the forwarded Subscribe (with no ack_id), it takes the default
	// path.  A new app then sends a message to verify end-to-end routing.
	Describe("old app subscribes via new relay", func() {
		It("new relay takes the default path and delivers messages to the old app", func() {
			if _, err := os.Stat(legacySDKMockPath); err != nil {
				Skip("sdk-mock-legacy binary not found at " + legacySDKMockPath)
			}

			relayPort := reservePort()

			replacements := map[string]string{
				"0.0.0.0:46490":         fmt.Sprintf("0.0.0.0:%d", relayPort),
				"http://localhost:46490": fmt.Sprintf("http://localhost:%d", relayPort),
			}

			relayConfig := writeTempConfig(tempDir, "./testdata/sub-ack-relay-config.yaml", "relay.yaml", replacements)
			relaySession, err := gexec.Start(
				exec.Command(slimPath, "--config", relayConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(relaySession, 5*time.Second)

			Eventually(relaySession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			// App A: old sdk-mock, subscriber.
			appAConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "legacy-app-a.yaml", replacements)
			appASession, err := gexec.Start(
				exec.Command(legacySDKMockPath,
					"--config", appAConfig,
					"--local-name", "a",
					"--remote-name", "b",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appASession, 5*time.Second)

			// New relay processes the old app's subscribe with default path
			// (no ack_id from old app, use_remote_ack=false).
			Eventually(relaySession.Out, 10*time.Second).Should(gbytes.Say("subscription: default ack path"))

			// App B: new sdk-mock, sender.
			appBConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "app-b.yaml", replacements)
			appBSession, err := gexec.Start(
				exec.Command(sdkMockPath,
					"--config", appBConfig,
					"--local-name", "b",
					"--remote-name", "a",
					"--message", "hey",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appBSession, 5*time.Second)

			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("Queueing reply"))
			Eventually(appBSession.Out, 10*time.Second).Should(gbytes.Say("hello from the a"))
		})
	})

	// ── old app ↔ old relay ───────────────────────────────────────────────────
	//
	// Baseline: both sides are pre-1.2.0.  No link negotiation, no remote ACK.
	// A second legacy app sends a message to verify routing still works.
	Describe("old app subscribes via old relay", func() {
		It("delivers messages between two old apps via the old relay", func() {
			if _, err := os.Stat(legacySDKMockPath); err != nil {
				Skip("sdk-mock-legacy binary not found at " + legacySDKMockPath)
			}

			relayPort := reservePort()

			replacements := map[string]string{
				"0.0.0.0:46490":         fmt.Sprintf("0.0.0.0:%d", relayPort),
				"http://localhost:46490": fmt.Sprintf("http://localhost:%d", relayPort),
			}

			relayConfig := writeTempConfig(tempDir, "./testdata/sub-ack-relay-config.yaml", "legacy-relay.yaml", replacements)
			relaySession, err := gexec.Start(
				exec.Command(legacySlimPath, "--config", relayConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(relaySession, 5*time.Second)

			Eventually(relaySession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			// App A: old sdk-mock, subscriber.
			appAConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "legacy-app-a.yaml", replacements)
			appASession, err := gexec.Start(
				exec.Command(legacySDKMockPath,
					"--config", appAConfig,
					"--local-name", "a",
					"--remote-name", "b",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appASession, 5*time.Second)

			// Both binaries are legacy: no ACK log lines to gate on.  Wait for
			// the relay to accept the connection and allow a brief propagation
			// delay before the sender starts.
			Eventually(relaySession.Out, 10*time.Second).Should(gbytes.Say("new connection received from remote"))
			time.Sleep(500 * time.Millisecond)

			// App B: old sdk-mock, sender.
			appBConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "legacy-app-b.yaml", replacements)
			appBSession, err := gexec.Start(
				exec.Command(legacySDKMockPath,
					"--config", appBConfig,
					"--local-name", "b",
					"--remote-name", "a",
					"--message", "hey",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appBSession, 5*time.Second)

			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("Queueing reply"))
			Eventually(appBSession.Out, 10*time.Second).Should(gbytes.Say("hello from the a"))
		})
	})

	// ── message delivery: old app ↔ new app through new relay ─────────────────
	//
	// App A is the legacy sdk-mock (subscriber), app B is the new sdk-mock
	// (sender).  The old app's embedded relay sends no ack_id; the new relay
	// takes the default path for that subscription.  Routing must work.
	Describe("message delivery: old app receives from new app via new relay", func() {
		It("routes a message from new app to old app through the new relay", func() {
			if _, err := os.Stat(legacySDKMockPath); err != nil {
				Skip("sdk-mock-legacy binary not found at " + legacySDKMockPath)
			}

			relayPort := reservePort()

			replacements := map[string]string{
				"0.0.0.0:46490":         fmt.Sprintf("0.0.0.0:%d", relayPort),
				"http://localhost:46490": fmt.Sprintf("http://localhost:%d", relayPort),
			}

			relayConfig := writeTempConfig(tempDir, "./testdata/sub-ack-relay-config.yaml", "relay.yaml", replacements)
			relaySession, err := gexec.Start(
				exec.Command(slimPath, "--config", relayConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(relaySession, 5*time.Second)

			Eventually(relaySession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			// App A: old sdk-mock, subscriber.
			appAConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "legacy-app-a.yaml", replacements)
			appASession, err := gexec.Start(
				exec.Command(legacySDKMockPath,
					"--config", appAConfig,
					"--local-name", "a",
					"--remote-name", "b",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appASession, 5*time.Second)

			// New relay takes the default path for the old app's subscribe (no ack_id).
			Eventually(relaySession.Out, 10*time.Second).Should(gbytes.Say("subscription: default ack path"))

			// App B: new sdk-mock, sender.
			appBConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "app-b.yaml", replacements)
			appBSession, err := gexec.Start(
				exec.Command(sdkMockPath,
					"--config", appBConfig,
					"--local-name", "b",
					"--remote-name", "a",
					"--message", "hey",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appBSession, 5*time.Second)

			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("Queueing reply"))
			Eventually(appBSession.Out, 10*time.Second).Should(gbytes.Say("hello from the a"))
		})
	})

	// ── message delivery: old app ↔ new app through old relay ─────────────────
	//
	// Same cross-version scenario but with the legacy relay as the hub.  Neither
	// app uses the remote-ack path.  Routing must still work end-to-end.
	Describe("message delivery: old app receives from new app via old relay", func() {
		It("routes a message from new app to old app through the old relay", func() {
			if _, err := os.Stat(legacySDKMockPath); err != nil {
				Skip("sdk-mock-legacy binary not found at " + legacySDKMockPath)
			}

			relayPort := reservePort()

			replacements := map[string]string{
				"0.0.0.0:46490":         fmt.Sprintf("0.0.0.0:%d", relayPort),
				"http://localhost:46490": fmt.Sprintf("http://localhost:%d", relayPort),
			}

			// Old relay as hub.
			relayConfig := writeTempConfig(tempDir, "./testdata/sub-ack-relay-config.yaml", "legacy-relay.yaml", replacements)
			relaySession, err := gexec.Start(
				exec.Command(legacySlimPath, "--config", relayConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(relaySession, 5*time.Second)

			Eventually(relaySession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			// App A: old sdk-mock, subscriber.
			appAConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "legacy-app-a.yaml", replacements)
			appASession, err := gexec.Start(
				exec.Command(legacySDKMockPath,
					"--config", appAConfig,
					"--local-name", "a",
					"--remote-name", "b",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appASession, 5*time.Second)

			// App A's embedded relay falls back to the default path (old relay
			// has no version).  Wait for its ACK log before starting the sender.
			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("subscription: default ack path"))

			// App B: new sdk-mock, sender.
			appBConfig := writeTempConfig(tempDir, "./testdata/sub-ack-app-config.yaml", "app-b.yaml", replacements)
			appBSession, err := gexec.Start(
				exec.Command(sdkMockPath,
					"--config", appBConfig,
					"--local-name", "b",
					"--remote-name", "a",
					"--message", "hey",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(appBSession, 5*time.Second)

			Eventually(appASession.Out, 10*time.Second).Should(gbytes.Say("Queueing reply"))
			Eventually(appBSession.Out, 10*time.Second).Should(gbytes.Say("hello from the a"))
		})
	})
})
