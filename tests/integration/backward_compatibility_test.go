// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

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

var _ = Describe("Backward Compatibility", func() {
	const (
		testSecret = "a-very-long-shared-secret-abcdef1234567890"
	)

	var (
		tempDir string
	)

	BeforeEach(func() {
		tempDir = newTempDir("slim-integration-backward-compat-")
	})

	AfterEach(func() {
		if tempDir != "" {
			_ = os.RemoveAll(tempDir)
			tempDir = ""
		}
	})

	// ====================================================================================
	// Tests on CURRENT SLIM node - All apps (mixed legacy/current) connect to current SLIM
	// ====================================================================================
	Describe("apps running on current SLIM node", func() {

		// Test 1a: Latest sender → Legacy receiver (P2P)
		Describe("latest sender communicating with legacy receiver over P2P", func() {
			It("should deliver messages bidirectionally", func() {
				slimPort := reservePort()

				replacements := map[string]string{
					"0.0.0.0:46480": fmt.Sprintf("0.0.0.0:%d", slimPort),
				}
				slimConfig := writeTempConfig(tempDir, "./testdata/backward-compat-server-config.yaml", "slim.yaml", replacements)

				// Start current SLIM node
				slimSession, err := gexec.Start(
					exec.Command(slimPath, "--config", slimConfig),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(slimSession, 5*time.Second)
				Eventually(slimSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

				// Start legacy receiver
				legacyReceiverSession, err := gexec.Start(
					exec.Command(
						legacyReceiverPath,
						"--local", "agntcy/ns/alice",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(legacyReceiverSession, 5*time.Second)
				Eventually(legacyReceiverSession.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

				// Start latest sender
				latestSenderSession, err := gexec.Start(
					exec.Command(
						senderPath,
						"--local", "agntcy/ns/bob",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
						"--session-type", "p2p",
						"--participants", "agntcy/ns/alice",
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(latestSenderSession, 5*time.Second)

				Eventually(latestSenderSession.Out, 15*time.Second).Should(gbytes.Say("Session .* established"))
				Eventually(latestSenderSession.Out, 10*time.Second).Should(gbytes.Say("Sending: Message 1"))
				Eventually(latestSenderSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 1 from agntcy/ns/alice.*"))
				Eventually(latestSenderSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 10 from agntcy/ns/alice.*"))
				Eventually(latestSenderSession.Out, 10*time.Second).Should(gbytes.Say("✓ All participants replied correctly"))
			})
		})

		// Test 2a: Legacy sender → Latest receiver (P2P)
		Describe("legacy sender communicating with latest receiver over P2P", func() {
			It("should deliver messages bidirectionally", func() {
				slimPort := reservePort()

				replacements := map[string]string{
					"0.0.0.0:46480": fmt.Sprintf("0.0.0.0:%d", slimPort),
				}
				slimConfig := writeTempConfig(tempDir, "./testdata/backward-compat-server-config.yaml", "slim.yaml", replacements)

				// Start current SLIM node
				slimSession, err := gexec.Start(
					exec.Command(slimPath, "--config", slimConfig),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(slimSession, 5*time.Second)
				Eventually(slimSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

				// Start latest receiver
				latestReceiverSession, err := gexec.Start(
					exec.Command(
						receiverPath,
						"--local", "agntcy/ns/alice",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(latestReceiverSession, 5*time.Second)
				Eventually(latestReceiverSession.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

				// Start legacy sender
				legacySenderSession, err := gexec.Start(
					exec.Command(
						legacySenderPath,
						"--local", "agntcy/ns/bob",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
						"--session-type", "p2p",
						"--participants", "agntcy/ns/alice",
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(legacySenderSession, 5*time.Second)

				Eventually(legacySenderSession.Out, 15*time.Second).Should(gbytes.Say("Session .* established"))
				Eventually(legacySenderSession.Out, 10*time.Second).Should(gbytes.Say("Sending: Message 1"))
				Eventually(legacySenderSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 1 from agntcy/ns/alice.*"))
				Eventually(legacySenderSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 10 from agntcy/ns/alice.*"))
				Eventually(legacySenderSession.Out, 10*time.Second).Should(gbytes.Say("✓ All participants replied correctly"))
			})
		})

		// Test 3a: Latest moderator with mixed participants (Group)
		Describe("latest moderator with mixed legacy and latest participants in group session", func() {
			It("should deliver group messages to all participants", func() {
				slimPort := reservePort()

				replacements := map[string]string{
					"0.0.0.0:46480": fmt.Sprintf("0.0.0.0:%d", slimPort),
				}
				slimConfig := writeTempConfig(tempDir, "./testdata/backward-compat-server-config.yaml", "slim.yaml", replacements)

				// Start current SLIM node
				slimSession, err := gexec.Start(
					exec.Command(slimPath, "--config", slimConfig),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(slimSession, 5*time.Second)
				Eventually(slimSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

				// Start latest receiver (participant1 - alice)
				latestParticipantSession, err := gexec.Start(
					exec.Command(
						receiverPath,
						"--local", "agntcy/ns/alice",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(latestParticipantSession, 5*time.Second)
				Eventually(latestParticipantSession.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

				// Start legacy receiver (participant2 - charlie)
				legacyParticipantSession, err := gexec.Start(
					exec.Command(
						legacyReceiverPath,
						"--local", "agntcy/ns/charlie",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(legacyParticipantSession, 5*time.Second)
				Eventually(legacyParticipantSession.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

				// Start latest moderator (sender - bob)
				latestModeratorSession, err := gexec.Start(
					exec.Command(
						senderPath,
						"--local", "agntcy/ns/bob",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
						"--session-type", "group",
						"--participants", "agntcy/ns/alice", "agntcy/ns/charlie",
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(latestModeratorSession, 5*time.Second)

				Eventually(latestModeratorSession.Out, 15*time.Second).Should(gbytes.Say("Session .* established"))
				Eventually(latestModeratorSession.Out, 15*time.Second).Should(gbytes.Say("agntcy/ns/alice.* joined session"))
				Eventually(latestModeratorSession.Out, 15*time.Second).Should(gbytes.Say("agntcy/ns/charlie.* joined session"))
				Eventually(latestModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Sending: Message 1"))
				Eventually(latestModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 1 from agntcy/ns/alice.*"))
				Eventually(latestModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/charlie.*: Message 1 from agntcy/ns/charlie.*"))
				Eventually(latestModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 10 from agntcy/ns/alice.*"))
				Eventually(latestModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/charlie.*: Message 10 from agntcy/ns/charlie.*"))
				Eventually(latestModeratorSession.Out, 15*time.Second).Should(gbytes.Say("✓ All participants replied correctly"))
			})
		})

		// Test 4a: Legacy moderator with mixed participants (Group)
		Describe("legacy moderator with mixed legacy and latest participants in group session", func() {
			It("should deliver group messages to all participants", func() {
				slimPort := reservePort()

				replacements := map[string]string{
					"0.0.0.0:46480": fmt.Sprintf("0.0.0.0:%d", slimPort),
				}
				slimConfig := writeTempConfig(tempDir, "./testdata/backward-compat-server-config.yaml", "slim.yaml", replacements)

				// Start current SLIM node
				slimSession, err := gexec.Start(
					exec.Command(slimPath, "--config", slimConfig),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(slimSession, 5*time.Second)
				Eventually(slimSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

				// Start legacy receiver (participant1 - alice)
				legacyParticipantSession, err := gexec.Start(
					exec.Command(
						legacyReceiverPath,
						"--local", "agntcy/ns/alice",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(legacyParticipantSession, 5*time.Second)
				Eventually(legacyParticipantSession.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

				// Start latest receiver (participant2 - charlie)
				latestParticipantSession, err := gexec.Start(
					exec.Command(
						receiverPath,
						"--local", "agntcy/ns/charlie",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(latestParticipantSession, 5*time.Second)
				Eventually(latestParticipantSession.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

				// Start legacy moderator (sender - bob)
				legacyModeratorSession, err := gexec.Start(
					exec.Command(
						legacySenderPath,
						"--local", "agntcy/ns/bob",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
						"--session-type", "group",
						"--participants", "agntcy/ns/alice", "agntcy/ns/charlie",
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(legacyModeratorSession, 5*time.Second)

				Eventually(legacyModeratorSession.Out, 15*time.Second).Should(gbytes.Say("Session .* established"))
				Eventually(legacyModeratorSession.Out, 15*time.Second).Should(gbytes.Say("agntcy/ns/alice.* joined session"))
				Eventually(legacyModeratorSession.Out, 15*time.Second).Should(gbytes.Say("agntcy/ns/charlie.* joined session"))
				Eventually(legacyModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Sending: Message 1"))
				Eventually(legacyModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 1 from agntcy/ns/alice.*"))
				Eventually(legacyModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/charlie.*: Message 1 from agntcy/ns/charlie.*"))
				Eventually(legacyModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 10 from agntcy/ns/alice.*"))
				Eventually(legacyModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/charlie.*: Message 10 from agntcy/ns/charlie.*"))
				Eventually(legacyModeratorSession.Out, 15*time.Second).Should(gbytes.Say("✓ All participants replied correctly"))
			})
		})
	})

	// ====================================================================================
	// Tests on LEGACY SLIM node - All apps (mixed legacy/current) connect to legacy SLIM
	// ====================================================================================
	Describe("apps running on legacy SLIM node", func() {

		// Test 1b: Latest sender → Legacy receiver (P2P)
		Describe("latest sender communicating with legacy receiver over P2P", func() {
			It("should deliver messages bidirectionally", func() {
				slimPort := reservePort()

				replacements := map[string]string{
					"0.0.0.0:46480": fmt.Sprintf("0.0.0.0:%d", slimPort),
				}
				slimConfig := writeTempConfig(tempDir, "./testdata/backward-compat-server-config.yaml", "slim.yaml", replacements)

				// Start legacy SLIM node
				slimSession, err := gexec.Start(
					exec.Command(legacySlimPath, "--config", slimConfig),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(slimSession, 5*time.Second)
				Eventually(slimSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

				// Start legacy receiver
				legacyReceiverSession, err := gexec.Start(
					exec.Command(
						legacyReceiverPath,
						"--local", "agntcy/ns/alice",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(legacyReceiverSession, 5*time.Second)
				Eventually(legacyReceiverSession.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

				// Start latest sender
				latestSenderSession, err := gexec.Start(
					exec.Command(
						senderPath,
						"--local", "agntcy/ns/bob",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
						"--session-type", "p2p",
						"--participants", "agntcy/ns/alice",
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(latestSenderSession, 5*time.Second)

				Eventually(latestSenderSession.Out, 15*time.Second).Should(gbytes.Say("Session .* established"))
				Eventually(latestSenderSession.Out, 10*time.Second).Should(gbytes.Say("Sending: Message 1"))
				Eventually(latestSenderSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 1 from agntcy/ns/alice.*"))
				Eventually(latestSenderSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 10 from agntcy/ns/alice.*"))
				Eventually(latestSenderSession.Out, 10*time.Second).Should(gbytes.Say("✓ All participants replied correctly"))
			})
		})

		// Test 2b: Legacy sender → Latest receiver (P2P)
		Describe("legacy sender communicating with latest receiver over P2P", func() {
			It("should deliver messages bidirectionally", func() {
				slimPort := reservePort()

				replacements := map[string]string{
					"0.0.0.0:46480": fmt.Sprintf("0.0.0.0:%d", slimPort),
				}
				slimConfig := writeTempConfig(tempDir, "./testdata/backward-compat-server-config.yaml", "slim.yaml", replacements)

				// Start legacy SLIM node
				slimSession, err := gexec.Start(
					exec.Command(legacySlimPath, "--config", slimConfig),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(slimSession, 5*time.Second)
				Eventually(slimSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

				// Start latest receiver
				latestReceiverSession, err := gexec.Start(
					exec.Command(
						receiverPath,
						"--local", "agntcy/ns/alice",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(latestReceiverSession, 5*time.Second)
				Eventually(latestReceiverSession.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

				// Start legacy sender
				legacySenderSession, err := gexec.Start(
					exec.Command(
						legacySenderPath,
						"--local", "agntcy/ns/bob",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
						"--session-type", "p2p",
						"--participants", "agntcy/ns/alice",
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(legacySenderSession, 5*time.Second)

				Eventually(legacySenderSession.Out, 15*time.Second).Should(gbytes.Say("Session .* established"))
				Eventually(legacySenderSession.Out, 10*time.Second).Should(gbytes.Say("Sending: Message 1"))
				Eventually(legacySenderSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 1 from agntcy/ns/alice.*"))
				Eventually(legacySenderSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 10 from agntcy/ns/alice.*"))
				Eventually(legacySenderSession.Out, 10*time.Second).Should(gbytes.Say("✓ All participants replied correctly"))
			})
		})

		// Test 3b: Latest moderator with mixed participants (Group)
		Describe("latest moderator with mixed legacy and latest participants in group session", func() {
			It("should deliver group messages to all participants", func() {
				slimPort := reservePort()

				replacements := map[string]string{
					"0.0.0.0:46480": fmt.Sprintf("0.0.0.0:%d", slimPort),
				}
				slimConfig := writeTempConfig(tempDir, "./testdata/backward-compat-server-config.yaml", "slim.yaml", replacements)

				// Start legacy SLIM node
				slimSession, err := gexec.Start(
					exec.Command(legacySlimPath, "--config", slimConfig),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(slimSession, 5*time.Second)
				Eventually(slimSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

				// Start latest receiver (participant1 - alice)
				latestParticipantSession, err := gexec.Start(
					exec.Command(
						receiverPath,
						"--local", "agntcy/ns/alice",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(latestParticipantSession, 5*time.Second)
				Eventually(latestParticipantSession.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

				// Start legacy receiver (participant2 - charlie)
				legacyParticipantSession, err := gexec.Start(
					exec.Command(
						legacyReceiverPath,
						"--local", "agntcy/ns/charlie",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(legacyParticipantSession, 5*time.Second)
				Eventually(legacyParticipantSession.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

				// Start latest moderator (sender - bob)
				latestModeratorSession, err := gexec.Start(
					exec.Command(
						senderPath,
						"--local", "agntcy/ns/bob",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
						"--session-type", "group",
						"--participants", "agntcy/ns/alice", "agntcy/ns/charlie",
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(latestModeratorSession, 5*time.Second)

				Eventually(latestModeratorSession.Out, 15*time.Second).Should(gbytes.Say("Session .* established"))
				Eventually(latestModeratorSession.Out, 15*time.Second).Should(gbytes.Say("agntcy/ns/alice.* joined session"))
				Eventually(latestModeratorSession.Out, 15*time.Second).Should(gbytes.Say("agntcy/ns/charlie.* joined session"))
				Eventually(latestModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Sending: Message 1"))
				Eventually(latestModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 1 from agntcy/ns/alice.*"))
				Eventually(latestModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/charlie.*: Message 1 from agntcy/ns/charlie.*"))
				Eventually(latestModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 10 from agntcy/ns/alice.*"))
				Eventually(latestModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/charlie.*: Message 10 from agntcy/ns/charlie.*"))
				Eventually(latestModeratorSession.Out, 15*time.Second).Should(gbytes.Say("✓ All participants replied correctly"))
			})
		})

		// Test 4b: Legacy moderator with mixed participants (Group)
		Describe("legacy moderator with mixed legacy and latest participants in group session", func() {
			It("should deliver group messages to all participants", func() {
				slimPort := reservePort()

				replacements := map[string]string{
					"0.0.0.0:46480": fmt.Sprintf("0.0.0.0:%d", slimPort),
				}
				slimConfig := writeTempConfig(tempDir, "./testdata/backward-compat-server-config.yaml", "slim.yaml", replacements)

				// Start legacy SLIM node
				slimSession, err := gexec.Start(
					exec.Command(legacySlimPath, "--config", slimConfig),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(slimSession, 5*time.Second)
				Eventually(slimSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

				// Start legacy receiver (participant1 - alice)
				legacyParticipantSession, err := gexec.Start(
					exec.Command(
						legacyReceiverPath,
						"--local", "agntcy/ns/alice",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(legacyParticipantSession, 5*time.Second)
				Eventually(legacyParticipantSession.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

				// Start latest receiver (participant2 - charlie)
				latestParticipantSession, err := gexec.Start(
					exec.Command(
						receiverPath,
						"--local", "agntcy/ns/charlie",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(latestParticipantSession, 5*time.Second)
				Eventually(latestParticipantSession.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

				// Start legacy moderator (sender - bob)
				legacyModeratorSession, err := gexec.Start(
					exec.Command(
						legacySenderPath,
						"--local", "agntcy/ns/bob",
						"--shared-secret", testSecret,
						"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
						"--session-type", "group",
						"--participants", "agntcy/ns/alice", "agntcy/ns/charlie",
					),
					GinkgoWriter, GinkgoWriter,
				)
				Expect(err).NotTo(HaveOccurred())
				defer terminateSession(legacyModeratorSession, 5*time.Second)

				Eventually(legacyModeratorSession.Out, 15*time.Second).Should(gbytes.Say("Session .* established"))
				Eventually(legacyModeratorSession.Out, 15*time.Second).Should(gbytes.Say("agntcy/ns/alice.* joined session"))
				Eventually(legacyModeratorSession.Out, 15*time.Second).Should(gbytes.Say("agntcy/ns/charlie.* joined session"))
				Eventually(legacyModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Sending: Message 1"))
				Eventually(legacyModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 1 from agntcy/ns/alice.*"))
				Eventually(legacyModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/charlie.*: Message 1 from agntcy/ns/charlie.*"))
				Eventually(legacyModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 10 from agntcy/ns/alice.*"))
				Eventually(legacyModeratorSession.Out, 10*time.Second).Should(gbytes.Say("Reply from agntcy/ns/charlie.*: Message 10 from agntcy/ns/charlie.*"))
				Eventually(legacyModeratorSession.Out, 15*time.Second).Should(gbytes.Say("✓ All participants replied correctly"))
			})
		})
	})
})
