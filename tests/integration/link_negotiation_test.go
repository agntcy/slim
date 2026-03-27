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

var _ = Describe("Link Negotiation", func() {
	var tempDir string

	BeforeEach(func() {
		tempDir = newTempDir("slim-integration-link-neg-")
	})

	AfterEach(func() {
		if tempDir != "" {
			_ = os.RemoveAll(tempDir)
			tempDir = ""
		}
	})

	// new node <--> new node: both sides should complete link negotiation.
	Describe("new node connects to new node", func() {
		It("completes link negotiation in both directions", func() {
			serverPort := reservePort()
			nodeBPort := reservePort()

			replacements := map[string]string{
				"0.0.0.0:46357":         fmt.Sprintf("0.0.0.0:%d", serverPort),
				"0.0.0.0:46481":         fmt.Sprintf("0.0.0.0:%d", nodeBPort),
				"http://localhost:46480": fmt.Sprintf("http://localhost:%d", serverPort),
			}
			serverConfig := writeTempConfig(tempDir, "./testdata/server.yaml", "server.yaml", replacements)
			nodeBConfig := writeTempConfig(tempDir, "./testdata/link-neg-node-with-client-config.yaml", "node-b.yaml", replacements)

			serverSession, err := gexec.Start(
				exec.Command(slimPath, "--config", serverConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(serverSession, 5*time.Second)

			Eventually(serverSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			nodeBSession, err := gexec.Start(
				exec.Command(slimPath, "--config", nodeBConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(nodeBSession, 5*time.Second)

			// Server (A) receives the negotiation request (is_reply=false).
			Eventually(serverSession.Out, 10*time.Second).Should(gbytes.Say("received link negotiation"))

			// Client (B) receives the reply (is_reply=true).
			Eventually(nodeBSession.Out, 10*time.Second).Should(gbytes.Say("received link negotiation"))

			// Neither process should have exited.
			Consistently(serverSession, 500*time.Millisecond).ShouldNot(gexec.Exit())
			Consistently(nodeBSession, 500*time.Millisecond).ShouldNot(gexec.Exit())
		})
	})

	// new client <--> old server: the new client sends link negotiation but the
	// old server (slim-v1.1.0) does not understand it and silently drops it.
	// The connection must still be established and remain stable.
	Describe("new client connects to old server", func() {
		It("establishes the connection even when the server does not reply to link negotiation", func() {
			serverPort := reservePort()

			replacements := map[string]string{
				"0.0.0.0:46357":         fmt.Sprintf("0.0.0.0:%d", serverPort),
				"http://localhost:46357": fmt.Sprintf("http://localhost:%d", serverPort),
			}

			// Old server: slim-v1.1.0, predates link negotiation.
			legacyServerConfig := writeTempConfig(tempDir, "./testdata/server.yaml", "legacy-server.yaml", replacements)
			legacyServerSession, err := gexec.Start(
				exec.Command(legacySlimPath, "--config", legacyServerConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(legacyServerSession, 5*time.Second)

			Eventually(legacyServerSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			// New client connects to old server; sends link negotiation that will never be answered.
			newClientConfig := writeTempConfig(tempDir, "./testdata/client.yaml", "new-client.yaml", replacements)
			newClientSession, err := gexec.Start(
				exec.Command(slimPath, "--config", newClientConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(newClientSession, 5*time.Second)

			// The new slim node must log that it initiated the connection.
			Eventually(newClientSession.Out, 15*time.Second).Should(gbytes.Say("new connection initiated locally"))

			// The process must keep running; an unanswered link negotiation is not fatal.
			Consistently(newClientSession, 2*time.Second).ShouldNot(gexec.Exit())
		})
	})

	Describe("two new nodes advertise their version during link negotiation", func() {
		It("logs remote_version after link negotiation", func() {
			serverPort := reservePort()
			nodeBPort := reservePort()

			replacements := map[string]string{
				"0.0.0.0:46357":         fmt.Sprintf("0.0.0.0:%d", serverPort),
				"0.0.0.0:46481":         fmt.Sprintf("0.0.0.0:%d", nodeBPort),
				"http://localhost:46480": fmt.Sprintf("http://localhost:%d", serverPort),
			}
			serverConfig := writeTempConfig(tempDir, "./testdata/server.yaml", "server-v120.yaml", replacements)
			nodeBConfig := writeTempConfig(tempDir, "./testdata/link-neg-node-with-client-config.yaml", "node-b-v120.yaml", replacements)

			serverSession, err := gexec.Start(
				exec.Command(slimPath, "--config", serverConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(serverSession, 5*time.Second)

			Eventually(serverSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			nodeBSession, err := gexec.Start(
				exec.Command(slimPath, "--config", nodeBConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(nodeBSession, 5*time.Second)

			// Server receives the negotiation request.  The log line contains a
			// remote_version= field but ANSI colour codes split the key from the
			// value, so we match the message text and then a semver pattern
			// (e.g. "1.3.0") which appears uninterrupted in the value portion.
			Eventually(serverSession.Out, 10*time.Second).Should(gbytes.Say("received link negotiation"))
			Eventually(serverSession.Out, 10*time.Second).Should(gbytes.Say(`\d+\.\d+\.\d+`))

			// Client receives the reply and logs the remote version.
			Eventually(nodeBSession.Out, 10*time.Second).Should(gbytes.Say("received link negotiation"))
			Eventually(nodeBSession.Out, 10*time.Second).Should(gbytes.Say(`\d+\.\d+\.\d+`))

			// Both processes must keep running.
			Consistently(serverSession, 500*time.Millisecond).ShouldNot(gexec.Exit())
			Consistently(nodeBSession, 500*time.Millisecond).ShouldNot(gexec.Exit())
		})
	})

	// old client <--> new server: the old client (slim-v1.1.0) connects but
	// never sends link negotiation.  The new server must accept the connection
	// and remain stable without ever receiving a negotiation message.
	Describe("old client connects to new server", func() {
		It("accepts the connection even when the client does not send link negotiation", func() {
			serverPort := reservePort()

			replacements := map[string]string{
				"0.0.0.0:46357":         fmt.Sprintf("0.0.0.0:%d", serverPort),
				"http://localhost:46357": fmt.Sprintf("http://localhost:%d", serverPort),
			}

			// New server with debug logging.
			newServerConfig := writeTempConfig(tempDir, "./testdata/server.yaml", "new-server.yaml", replacements)
			newServerSession, err := gexec.Start(
				exec.Command(slimPath, "--config", newServerConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(newServerSession, 5*time.Second)

			Eventually(newServerSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			// Old client: slim-v1.1.0, connects but never sends link negotiation.
			legacyClientConfig := writeTempConfig(tempDir, "./testdata/client.yaml", "legacy-client.yaml", replacements)
			legacyClientSession, err := gexec.Start(
				exec.Command(legacySlimPath, "--config", legacyClientConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(legacyClientSession, 5*time.Second)

			// New server must log that it accepted the incoming connection.
			Eventually(newServerSession.Out, 10*time.Second).Should(gbytes.Say("new connection received from remote"))

			// Server must keep running; missing link negotiation is not fatal.
			Consistently(newServerSession, 2*time.Second).ShouldNot(gexec.Exit())
		})
	})
})
