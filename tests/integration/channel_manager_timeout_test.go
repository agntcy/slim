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

var _ = Describe("Group management through channel manager with timeout", func() {
	var (
		slimNodeSession       *gexec.Session
		channelManagerSession *gexec.Session

		tempDir              string
		serverAConfig        string
		channelManagerConfig string
		channelManagerPort   int
	)

	const channelName = "org/default/test-channel-timeout"

	BeforeEach(func() {
		fmt.Fprintf(GinkgoWriter, "[integration] Start: %s\n", CurrentSpecReport().FullText())

		dataPlaneAPort := reservePort()
		channelManagerPort = reservePort()

		replacements := map[string]string{
			"0.0.0.0:46357":          fmt.Sprintf("0.0.0.0:%d", dataPlaneAPort),
			"http://localhost:46357": fmt.Sprintf("http://localhost:%d", dataPlaneAPort),
			"http://127.0.0.1:46357": fmt.Sprintf("http://127.0.0.1:%d", dataPlaneAPort),
			"0.0.0.0:10356":          fmt.Sprintf("0.0.0.0:%d", channelManagerPort),
		}

		tempDir = newTempDir("slim-integration-gm-timeout-")
		serverAConfig = writeTempConfig(tempDir, "./testdata/server.yaml", "server-a-config.yaml", replacements)
		channelManagerConfig = writeTempConfig(tempDir, "./testdata/channel-manager-config.yaml", "channel-manager-config.yaml", replacements)

		// start a SLIM node
		var errNode error
		slimNodeSession, errNode = gexec.Start(
			exec.Command(slimPath, "--config", serverAConfig),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errNode).NotTo(HaveOccurred())

		// wait for SLIM node to start
		time.Sleep(2000 * time.Millisecond)

		// start channel manager
		var errCM error
		channelManagerSession, errCM = gexec.Start(
			exec.Command(channelManagerPath, "--config-file", channelManagerConfig),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errCM).NotTo(HaveOccurred())

		// wait for channel manager to connect and start gRPC server
		Eventually(channelManagerSession.Out, 15*time.Second).Should(gbytes.Say("Starting gRPC server"))
	})

	AfterEach(func() {
		fmt.Fprintf(GinkgoWriter, "[integration] End: %s\n", CurrentSpecReport().FullText())

		// terminate channel manager
		terminateSession(channelManagerSession, 30*time.Second)

		// terminate SLIM node
		terminateSession(slimNodeSession, 30*time.Second)

		if tempDir != "" {
			_ = os.RemoveAll(tempDir)
			tempDir = ""
		}
	})

	Describe("group management with channel manager with timeout", func() {
		It("SLIM node creates channel, adds nonexistent participant", func() {
			cmEndpoint := fmt.Sprintf("127.0.0.1:%d", channelManagerPort)

			// Create channel via channel manager
			createOutput := runCombinedOutputWithRetry(10*time.Second, func() *exec.Cmd {
				return exec.Command(
					slimctlPath,
					"cm", "create-channel", channelName,
					"-s", cmEndpoint,
				)
			})
			Expect(createOutput).NotTo(BeEmpty())

			participantA := "org/default/a"

			// Invite nonexistent clientA to the channel — should fail
			addClientAOutput, errA := exec.Command(
				slimctlPath,
				"cm", "add-participant", channelName, participantA,
				"-s", cmEndpoint,
			).CombinedOutput()

			time.Sleep(2000 * time.Millisecond)

			Expect(errA).To(HaveOccurred())
			Expect(string(addClientAOutput)).To(ContainSubstring("failed to invite participant"))
		})
	})
})
