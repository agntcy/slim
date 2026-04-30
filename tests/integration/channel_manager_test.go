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

var _ = Describe("Group management through channel manager", func() {
	var (
		slimNodeSession       *gexec.Session
		channelManagerSession *gexec.Session
		clientASession        *gexec.Session
		clientBSession        *gexec.Session
		clientCSession        *gexec.Session

		tempDir              string
		serverAConfig        string
		clientAConfig        string
		clientBConfig        string
		clientCConfig        string
		channelManagerConfig string
		channelManagerPort   int
	)

	const channelName = "org/default/test-channel"

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

		tempDir = newTempDir("slim-integration-gm-")
		serverAConfig = writeTempConfig(tempDir, "./testdata/server.yaml", "server-a-config.yaml", replacements)
		clientAConfig = writeTempConfig(tempDir, "./testdata/client.yaml", "client-a-config.yaml", replacements)
		clientBConfig = writeTempConfig(tempDir, "./testdata/client.yaml", "client-b-config.yaml", replacements)
		clientCConfig = writeTempConfig(tempDir, "./testdata/client.yaml", "client-c-config.yaml", replacements)
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

		// start clients
		var errClientA, errClientC error
		clientASession, errClientA = gexec.Start(
			exec.Command(clientPath,
				"--config", clientAConfig,
				"--local-name", "org/default/a", "--secret", "group-abcdef-12345678901234567890",
			),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errClientA).NotTo(HaveOccurred())

		var errClientB error
		clientBSession, errClientB = gexec.Start(
			exec.Command(clientPath,
				"--config", clientBConfig,
				"--local-name", "org/default/b", "--secret", "group-abcdef-12345678901234567890",
			),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errClientB).NotTo(HaveOccurred())

		clientCSession, errClientC = gexec.Start(
			exec.Command(clientPath,
				"--config", clientCConfig,
				"--local-name", "org/default/c",
				"--secret", "group-abcdef-12345678901234567890",
				"--message", "hey there, I am c!",
			),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errClientC).NotTo(HaveOccurred())

		// wait for clients to connect
		time.Sleep(1000 * time.Millisecond)
	})

	AfterEach(func() {
		fmt.Fprintf(GinkgoWriter, "[integration] End: %s\n", CurrentSpecReport().FullText())

		// terminate clients
		terminateSession(clientASession, 2*time.Second)
		terminateSession(clientBSession, 2*time.Second)
		terminateSession(clientCSession, 2*time.Second)

		// terminate channel manager
		terminateSession(channelManagerSession, 30*time.Second)

		// terminate SLIM node
		terminateSession(slimNodeSession, 30*time.Second)

		if tempDir != "" {
			_ = os.RemoveAll(tempDir)
			tempDir = ""
		}
	})

	Describe("group management with channel manager", func() {
		It("SLIM node creates channel, adds participant, removes participant and deletes channel", func() {
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
			Expect(string(createOutput)).To(ContainSubstring("created successfully"))
			fmt.Fprintf(GinkgoWriter, "Create channel output: %s\n", string(createOutput))

			participantA := "org/default/a"
			participantB := "org/default/b"
			participantC := "org/default/c"

			// Add clientA to the channel
			addClientAOutput := runCombinedOutputWithRetry(10*time.Second, func() *exec.Cmd {
				return exec.Command(
					slimctlPath,
					"cm", "add-participant", channelName, participantA,
					"-s", cmEndpoint,
				)
			})
			Expect(addClientAOutput).NotTo(BeEmpty())
			Expect(string(addClientAOutput)).To(ContainSubstring("added to channel"))

			Eventually(clientASession, 15*time.Second).Should(gbytes.Say(MsgSessionHandlerTaskStarted))

			// Add clientB to the channel
			addClientBOutput := runCombinedOutputWithRetry(10*time.Second, func() *exec.Cmd {
				return exec.Command(
					slimctlPath,
					"cm", "add-participant", channelName, participantB,
					"-s", cmEndpoint,
				)
			})
			Expect(addClientBOutput).NotTo(BeEmpty())

			Eventually(clientBSession, 15*time.Second).Should(gbytes.Say(MsgSessionHandlerTaskStarted))

			// Add clientC to the channel
			addClientCOutput := runCombinedOutputWithRetry(10*time.Second, func() *exec.Cmd {
				return exec.Command(
					slimctlPath,
					"cm", "add-participant", channelName, participantC,
					"-s", cmEndpoint,
				)
			})
			Expect(addClientCOutput).NotTo(BeEmpty())

			Eventually(clientCSession, 15*time.Second).Should(gbytes.Say(MsgSessionHandlerTaskStarted))

			// Test communication: clientC's message should be received by both clientA and clientB
			Eventually(clientASession.Out, 10*time.Second).
				Should(gbytes.Say(MsgTestClientCMessage))

			Eventually(clientBSession.Out, 10*time.Second).
				Should(gbytes.Say(MsgTestClientCMessage))

			// Remove participant c from the channel
			deleteParticipantOutput := runCombinedOutputWithRetry(10*time.Second, func() *exec.Cmd {
				return exec.Command(
					slimctlPath,
					"cm", "delete-participant", channelName, participantC,
					"-s", cmEndpoint,
				)
			})
			Expect(deleteParticipantOutput).NotTo(BeEmpty())

			Eventually(clientCSession, 15*time.Second).Should(gbytes.Say(MsgSessionClosed))

			// Delete channel
			deleteChannelOutput := runCombinedOutputWithRetry(10*time.Second, func() *exec.Cmd {
				return exec.Command(
					slimctlPath,
					"cm", "delete-channel", channelName,
					"-s", cmEndpoint,
				)
			})
			Expect(deleteChannelOutput).NotTo(BeEmpty())

			Eventually(clientASession, 15*time.Second).Should(gbytes.Say(MsgSessionClosed))
		})
	})
})
