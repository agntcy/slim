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

// pathGetter is a function type that returns an executable path at runtime
type pathGetter func() string

// testP2PSession is a helper function that runs a P2P backward compatibility test.
// It starts a SLIM node, a receiver, and a sender with the specified executable paths.
func testP2PSession(tempDir string, getSlimPath, getSenderPath, getReceiverPath pathGetter) {
	slimPort := reservePort()

	replacements := map[string]string{
		"0.0.0.0:46357": fmt.Sprintf("0.0.0.0:%d", slimPort),
	}
	slimConfig := writeTempConfig(tempDir, "./testdata/server.yaml", "slim.yaml", replacements)

	// Start SLIM node
	slimSession, err := gexec.Start(
		exec.Command(getSlimPath(), "--config", slimConfig),
		GinkgoWriter, GinkgoWriter,
	)
	Expect(err).NotTo(HaveOccurred())
	defer terminateSession(slimSession, 5*time.Second)
	Eventually(slimSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

	// Start receiver
	receiverSession, err := gexec.Start(
		exec.Command(
			getReceiverPath(),
			"--local", "agntcy/ns/alice",
			"--shared-secret", "a-very-long-shared-secret-abcdef1234567890",
			"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
		),
		GinkgoWriter, GinkgoWriter,
	)
	Expect(err).NotTo(HaveOccurred())
	defer terminateSession(receiverSession, 5*time.Second)
	Eventually(receiverSession.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

	// Start sender
	senderSession, err := gexec.Start(
		exec.Command(
			getSenderPath(),
			"--local", "agntcy/ns/bob",
			"--shared-secret", "a-very-long-shared-secret-abcdef1234567890",
			"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
			"--session-type", "p2p",
			"--participants", "agntcy/ns/alice",
		),
		GinkgoWriter, GinkgoWriter,
	)
	Expect(err).NotTo(HaveOccurred())
	defer terminateSession(senderSession, 5*time.Second)

	Eventually(senderSession.Out, 15*time.Second).Should(gbytes.Say("Session .* established"))
	Eventually(senderSession.Out, 15*time.Second).Should(gbytes.Say("Sending: Message 1"))
	Eventually(senderSession.Out, 15*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 1 from agntcy/ns/alice.*"))
	Eventually(senderSession.Out, 15*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice.*: Message 10 from agntcy/ns/alice.*"))
	Eventually(senderSession.Out, 15*time.Second).Should(gbytes.Say("✓ All participants replied correctly"))
}

// testGroupSession is a helper function that runs a Group backward compatibility test.
// It starts a SLIM node, two receivers (participants), and a sender (moderator) with the specified executable paths.
func testGroupSession(tempDir string, getSlimPath, getModeratorPath, getParticipant1Path, getParticipant2Path pathGetter) {
	slimPort := reservePort()

	replacements := map[string]string{
		"0.0.0.0:46357": fmt.Sprintf("0.0.0.0:%d", slimPort),
	}
	slimConfig := writeTempConfig(tempDir, "./testdata/server.yaml", "slim.yaml", replacements)

	// Start SLIM node
	slimSession, err := gexec.Start(
		exec.Command(getSlimPath(), "--config", slimConfig),
		GinkgoWriter, GinkgoWriter,
	)
	Expect(err).NotTo(HaveOccurred())
	defer terminateSession(slimSession, 5*time.Second)
	Eventually(slimSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

	// Start participant 1 (alice)
	participant1Session, err := gexec.Start(
		exec.Command(
			getParticipant1Path(),
			"--local", "agntcy/ns/alice",
			"--shared-secret", "a-very-long-shared-secret-abcdef1234567890",
			"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
		),
		GinkgoWriter, GinkgoWriter,
	)
	Expect(err).NotTo(HaveOccurred())
	defer terminateSession(participant1Session, 5*time.Second)
	Eventually(participant1Session.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

	// Start participant 2 (charlie)
	participant2Session, err := gexec.Start(
		exec.Command(
			getParticipant2Path(),
			"--local", "agntcy/ns/charlie",
			"--shared-secret", "a-very-long-shared-secret-abcdef1234567890",
			"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
		),
		GinkgoWriter, GinkgoWriter,
	)
	Expect(err).NotTo(HaveOccurred())
	defer terminateSession(participant2Session, 5*time.Second)
	Eventually(participant2Session.Out, 10*time.Second).Should(gbytes.Say("Waiting for incoming session"))

	// Start moderator (bob)
	moderatorSession, err := gexec.Start(
		exec.Command(
			getModeratorPath(),
			"--local", "agntcy/ns/bob",
			"--shared-secret", "a-very-long-shared-secret-abcdef1234567890",
			"--slim", fmt.Sprintf("http://localhost:%d", slimPort),
			"--session-type", "group",
			"--participants", "agntcy/ns/alice", "agntcy/ns/charlie",
		),
		GinkgoWriter, GinkgoWriter,
	)
	Expect(err).NotTo(HaveOccurred())
	defer terminateSession(moderatorSession, 5*time.Second)

	Eventually(moderatorSession.Out, 15*time.Second).Should(gbytes.Say("Session .* established"))
	Eventually(moderatorSession.Out, 15*time.Second).Should(gbytes.Say("agntcy/ns/alice.* joined session"))
	Eventually(moderatorSession.Out, 15*time.Second).Should(gbytes.Say("agntcy/ns/charlie.* joined session"))
	Eventually(moderatorSession.Out, 15*time.Second).Should(gbytes.Say("Sending: Message 1"))
	Eventually(moderatorSession.Out, 15*time.Second).Should(gbytes.Say("Reply from agntcy/ns/alice"))
	Eventually(moderatorSession.Out, 15*time.Second).Should(gbytes.Say("Reply from agntcy/ns/charlie"))
	Eventually(moderatorSession.Out, 15*time.Second).Should(gbytes.Say("✓ All participants replied correctly"))
}

var _ = Describe("Backward Compatibility", func() {
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

	// P2P session tests with different combinations of SLIM and app versions
	DescribeTable("P2P sessions",
		func(getSlimPath, getSenderPath, getReceiverPath pathGetter) {
			testP2PSession(tempDir, getSlimPath, getSenderPath, getReceiverPath)
		},
		// Tests on CURRENT SLIM node
		Entry("current SLIM: latest sender → legacy receiver", func() string { return slimPath }, func() string { return senderPath }, func() string { return legacyReceiverPath }),
		Entry("current SLIM: legacy sender → latest receiver", func() string { return slimPath }, func() string { return legacySenderPath }, func() string { return receiverPath }),
		Entry("current SLIM: legacy sender → legacy receiver", func() string { return slimPath }, func() string { return legacySenderPath }, func() string { return legacyReceiverPath }),
		// Tests on LEGACY SLIM node
		Entry("legacy SLIM: latest sender → legacy receiver", func() string { return legacySlimPath }, func() string { return senderPath }, func() string { return legacyReceiverPath }),
		Entry("legacy SLIM: legacy sender → latest receiver", func() string { return legacySlimPath }, func() string { return legacySenderPath }, func() string { return receiverPath }),
		Entry("legacy SLIM: latest sender → latest receiver", func() string { return legacySlimPath }, func() string { return senderPath }, func() string { return receiverPath }),
	)

	// Group session tests with different combinations of SLIM and app versions
	DescribeTable("Group sessions",
		func(getSlimPath, getModeratorPath, getParticipant1Path, getParticipant2Path pathGetter) {
			testGroupSession(tempDir, getSlimPath, getModeratorPath, getParticipant1Path, getParticipant2Path)
		},
		// Tests on CURRENT SLIM node
		Entry("current SLIM: latest moderator with mixed participants", func() string { return slimPath }, func() string { return senderPath }, func() string { return receiverPath }, func() string { return legacyReceiverPath }),
		Entry("current SLIM: legacy moderator with mixed participants", func() string { return slimPath }, func() string { return legacySenderPath }, func() string { return receiverPath }, func() string { return legacyReceiverPath }),
		// Tests on LEGACY SLIM node
		Entry("legacy SLIM: latest moderator with mixed participants", func() string { return legacySlimPath }, func() string { return senderPath }, func() string { return receiverPath }, func() string { return legacyReceiverPath }),
		Entry("legacy SLIM: legacy moderator with mixed participants", func() string { return legacySlimPath }, func() string { return legacySenderPath }, func() string { return receiverPath }, func() string { return legacyReceiverPath }),
	)
})
