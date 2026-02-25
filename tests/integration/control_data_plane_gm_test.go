// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
	"fmt"
	"os"
	"os/exec"
	"strings"
	"time"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	"github.com/onsi/gomega/gbytes"
	"github.com/onsi/gomega/gexec"
)

var _ = Describe("Group management through control plane", func() {
	var (
		controlPlaneSession *gexec.Session
		slimNodeSession     *gexec.Session
		moderatorSession    *gexec.Session
		clientASession      *gexec.Session
		clientCSession      *gexec.Session
		controlPlaneDBPath  string

		tempDir               string
		serverAConfig         string
		clientAConfig         string
		clientCConfig         string
		moderatorConfig       string
		controlPlaneConfig    string
		controlPlaneNorthPort int
		controlPlaneSouthPort int
	)

	BeforeEach(func() {
		fmt.Fprintf(GinkgoWriter, "[integration] Start: %s\n", CurrentSpecReport().FullText())

		dataPlaneAPort := reservePort()
		controlPlaneNorthPort = reservePort()
		controlPlaneSouthPort = reservePort()

		replacements := map[string]string{
			"0.0.0.0:46357":          fmt.Sprintf("0.0.0.0:%d", dataPlaneAPort),
			"http://localhost:46357": fmt.Sprintf("http://localhost:%d", dataPlaneAPort),
			"http://127.0.0.1:46357": fmt.Sprintf("http://127.0.0.1:%d", dataPlaneAPort),
			"http://127.0.0.1:50051": fmt.Sprintf("http://127.0.0.1:%d", controlPlaneNorthPort),
			"http://127.0.0.1:50052": fmt.Sprintf("http://127.0.0.1:%d", controlPlaneSouthPort),
			"httpPort: 50051":        fmt.Sprintf("httpPort: %d", controlPlaneNorthPort),
			"httpPort: 50052":        fmt.Sprintf("httpPort: %d", controlPlaneSouthPort),
		}

		tempDir = newTempDir("slim-integration-gm-")
		serverAConfig = writeTempConfig(tempDir, "./testdata/server-a-config-cp.yaml", "server-a-config-cp.yaml", replacements)
		clientAConfig = writeTempConfig(tempDir, "./testdata/client-a-config.yaml", "client-a-config.yaml", replacements)
		clientCConfig = writeTempConfig(tempDir, "./testdata/client-c-config.yaml", "client-c-config.yaml", replacements)
		moderatorConfig = writeTempConfig(tempDir, "./testdata/moderator-config.yaml", "moderator-config.yaml", replacements)
		controlPlaneConfig = writeTempConfig(tempDir, "./testdata/control-plane-config.yaml", "control-plane-config.yaml", replacements)

		// start control plane
		var errCP error
		var err error
		var controlPlaneDB *os.File
		controlPlaneDB, err = os.CreateTemp("", "controlplane-*.db")
		Expect(err).NotTo(HaveOccurred())
		controlPlaneDBPath = controlPlaneDB.Name()
		Expect(controlPlaneDB.Close()).To(Succeed())

		controlPlaneCmd := exec.Command(controlPlanePath, "--config", controlPlaneConfig)
		controlPlaneCmd.Env = append(os.Environ(), "DATABASE_FILEPATH="+controlPlaneDBPath)
		controlPlaneSession, errCP = gexec.Start(
			controlPlaneCmd,
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errCP).NotTo(HaveOccurred())
		Eventually(controlPlaneSession.Out, 15*time.Second).Should(gbytes.Say("Northbound API Service is listening on"))

		// start a SLIM node
		var errNode error
		slimNodeSession, errNode = gexec.Start(
			exec.Command(slimPath, "--config", serverAConfig),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errNode).NotTo(HaveOccurred())

		// wait for services to start
		time.Sleep(2000 * time.Millisecond)
		Eventually(slimNodeSession.Out, 15*time.Second).Should(gbytes.Say("connected to control plane"))

		// start moderator
		var errModerator error
		moderatorSession, errModerator = gexec.Start(
			exec.Command(clientPath, "--config", moderatorConfig, "--local-name", "org/default/moderator1", "--secret", "group-abcdef-12345678901234567890"),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errModerator).NotTo(HaveOccurred())

		// wait for moderator to connect
		time.Sleep(1000 * time.Millisecond)

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
		terminateSession(clientCSession, 2*time.Second)

		// terminate moderator
		terminateSession(moderatorSession, 30*time.Second)

		// terminate SLIM node plane
		terminateSession(slimNodeSession, 30*time.Second)

		// terminate control plane
		terminateSession(controlPlaneSession, 30*time.Second)

		// delete control plane database file
		if controlPlaneDBPath != "" {
			err := os.Remove(controlPlaneDBPath)
			Expect(err).NotTo(HaveOccurred())
			controlPlaneDBPath = ""
		}

		if tempDir != "" {
			_ = os.RemoveAll(tempDir)
			tempDir = ""
		}
	})

	Describe("group management with control plane", func() {
		var channelName string
		It("SLIM node creates channel, adds participant, removes participant and deletes channel", func() {
			addChannelOutput, err := exec.Command(
				slimctlPath,
				"-s", fmt.Sprintf("127.0.0.1:%d", controlPlaneNorthPort), "--tls-insecure",
				"c", "channel", "create",
				"moderators=org/default/moderator1/0",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred())
			Expect(addChannelOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say(MsgCreateChannelRequest))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(fmt.Sprintf(MsgChannelCreated, ".+", true)))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(MsgChannelSavedSuccessfully))

			// CombinedOutput is "Received response org/default/moderator1/0-... \n"
			// Split channelID by spaces and get the chunk starting with org/default/moderator1/0
			for _, name := range strings.Fields(string(addChannelOutput)) {
				if strings.HasPrefix(name, "org/default") {
					channelName = name
					break
				}
			}

			participantA := "org/default/a"
			participantC := "org/default/c"

			// Invite clientA to the channel
			addClientAOutput, errA := exec.Command(
				slimctlPath,
				"-s", fmt.Sprintf("127.0.0.1:%d", controlPlaneNorthPort), "--tls-insecure",
				"c", "participant", "add",
				participantA,
				"--channel-id", channelName,
			).CombinedOutput()

			Expect(errA).NotTo(HaveOccurred())
			Expect(addClientAOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say(MsgParticipantAddRequest))
			Eventually(clientASession, 15*time.Second).Should(gbytes.Say(MsgSessionHandlerTaskStarted))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(fmt.Sprintf(MsgParticipantAdded, true)))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(MsgChannelUpdatedParticipantAdded))

			// Invite clientC to the channel
			addClientCOutput, errB := exec.Command(
				slimctlPath,
				"-s", fmt.Sprintf("127.0.0.1:%d", controlPlaneNorthPort), "--tls-insecure",
				"c", "participant", "add",
				participantC,
				"--channel-id", channelName,
			).CombinedOutput()

			Expect(errB).NotTo(HaveOccurred())
			Expect(addClientCOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say(MsgParticipantAddRequest))
			Eventually(clientCSession, 15*time.Second).Should(gbytes.Say(MsgSessionHandlerTaskStarted))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(fmt.Sprintf(MsgParticipantAdded, true)))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(MsgChannelUpdatedParticipantAdded))

			// Test communication between clientA and clientC
			Eventually(moderatorSession.Out, 10*time.Second).
				Should(gbytes.Say(MsgTestClientCMessage))

			Eventually(clientASession.Out, 10*time.Second).
				Should(gbytes.Say(MsgTestClientCMessage))

			// Remove participant c from the channel
			deleteParticipantOutput, errP := exec.Command(
				slimctlPath,
				"-s", fmt.Sprintf("127.0.0.1:%d", controlPlaneNorthPorts), "--tls-insecure",
				"c", "participant", "delete",
				participantC,
				"--channel-id", channelName,
			).CombinedOutput()

			Expect(errP).NotTo(HaveOccurred())
			Expect(deleteParticipantOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say(MsgParticipantDeleteRequest))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(fmt.Sprintf(MsgParticipantDeleted, true)))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(MsgChannelUpdatedParticipantDeleted))
			Eventually(clientCSession, 15*time.Second).Should(gbytes.Say(MsgSessionClosed))

			deleteChannelOutput := runCombinedOutputWithRetry(10*time.Second, func() *exec.Cmd {
				return exec.Command(
					slimctlPath,
					"-s", fmt.Sprintf("127.0.0.1:%d", controlPlaneNorthPort),
					"c", "channel", "delete",
					channelName,
				)
			})
			Expect(deleteChannelOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say(MsgChannelDeleteRequest))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(fmt.Sprintf(MsgAckChannelDeleted, channelName, true)))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(MsgChannelDeletedSuccessfully))
			Eventually(clientASession, 15*time.Second).Should(gbytes.Say(MsgSessionClosed))
			Eventually(moderatorSession, 15*time.Second).Should(gbytes.Say(MsgSessionClosed))
		})
	})
})
