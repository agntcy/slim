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

var _ = Describe("Route Recovery", func() {
	// A stable UUID v4 used to identify node B across reconnects.  Must match
	// the version nibble (4) and variant bits (8-b) required by is_valid_uuid_v4.
	const stableLinkID = "a1b2c3d4-e5f6-4789-abcd-ef1234567890"

	var tempDir string

	BeforeEach(func() {
		tempDir = newTempDir("slim-integration-route-recovery-")
	})

	AfterEach(func() {
		if tempDir != "" {
			_ = os.RemoveAll(tempDir)
			tempDir = ""
		}
	})

	// Server-side recovery: when an incoming peer reconnects with the same link_id
	// within the recovery window, the server must restore all routing state without
	// notifying the control plane that the connection was lost.
	Describe("server-side route recovery", func() {
		It("restores routing state when an incoming peer reconnects with the same link_id", func() {
			serverPort := reservePort()

			replacements := map[string]string{
				"0.0.0.0:46490":         fmt.Sprintf("0.0.0.0:%d", serverPort),
				"http://localhost:46490": fmt.Sprintf("http://localhost:%d", serverPort),
				"LINK_ID_PLACEHOLDER":   stableLinkID,
			}

			serverConfig := writeTempConfig(tempDir, "./testdata/route-recovery-server-config.yaml", "recovery-server.yaml", replacements)
			clientConfig := writeTempConfig(tempDir, "./testdata/route-recovery-client-config.yaml", "recovery-client.yaml", replacements)

			// Start the server (incoming side — exercises server-side recovery).
			serverSession, err := gexec.Start(
				exec.Command(slimPath, "--config", serverConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(serverSession, 5*time.Second)

			Eventually(serverSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			// Start the client (node B) with the stable link_id; it connects to the server.
			clientSession, err := gexec.Start(
				exec.Command(slimPath, "--config", clientConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())

			// Wait for link negotiation to complete on the server side.
			Eventually(serverSession.Out, 10*time.Second).Should(gbytes.Say("received link negotiation"))

			// Terminate the client — triggers server-side recovery state storage.
			terminateSession(clientSession, 5*time.Second)

			// The server must log that it preserved routing state for the dropped connection
			// instead of notifying the control plane immediately.
			Eventually(serverSession.Out, 10*time.Second).Should(gbytes.Say("connection lost, storing recovery state"))

			// Restart the client with the identical stable link_id.
			clientSession2, err := gexec.Start(
				exec.Command(slimPath, "--config", clientConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())
			defer terminateSession(clientSession2, 5*time.Second)

			// The server must log that it recovered routes for the reconnected peer.
			Eventually(serverSession.Out, 10*time.Second).Should(gbytes.Say("recovering routes for reconnected peer"))

			// Both processes must keep running after recovery.
			Consistently(serverSession, 500*time.Millisecond).ShouldNot(gexec.Exit())
			Consistently(clientSession2, 500*time.Millisecond).ShouldNot(gexec.Exit())
		})
	})
})
