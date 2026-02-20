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

var _ = Describe("Group management through control plane with timeout", func() {
	var (
		controlPlaneSession *gexec.Session
		slimNodeSession     *gexec.Session
		moderatorSession    *gexec.Session
		controlPlaneDBPath  string

		tempDir               string
		serverAConfig         string
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

		tempDir = newTempDir("slim-integration-gm-timeout-")
		serverAConfig = writeTempConfig(tempDir, "./testdata/server-a-config-cp.yaml", "server-a-config-cp.yaml", replacements)
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
	})

	AfterEach(func() {
		fmt.Fprintf(GinkgoWriter, "[integration] End: %s\n", CurrentSpecReport().FullText())

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

	Describe("group management with control plane with timeout", func() {
		var channelName string
		It("SLIM node creates channel, adds nonexistent participant", func() {
			addChannelOutput := runCombinedOutputWithRetry(10*time.Second, func() *exec.Cmd {
				return exec.Command(
					slimctlPath,
					"c", "channel", "create",
					"moderators=org/default/moderator1/0",
					"-s", fmt.Sprintf("127.0.0.1:%d", controlPlaneNorthPort),
				)
			})
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

			// Invite clientA to the channel
			addClientAOutput, errA := exec.Command(
				slimctlPath,
				"c", "participant", "add",
				participantA,
				"--channel-id", channelName,
				"-s", fmt.Sprintf("127.0.0.1:%d", controlPlaneNorthPort),
			).CombinedOutput()

			time.Sleep(2000 * time.Millisecond)

			Expect(errA).To(HaveOccurred())
			Expect(string(addClientAOutput)).To(ContainSubstring("failed to add participants: unsuccessful response"))
			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say(MsgParticipantAddRequest))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(fmt.Sprintf(MsgParticipantAdded, false)))
		})
	})
})
