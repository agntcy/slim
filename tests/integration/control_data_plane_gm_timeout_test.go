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
	)

	BeforeEach(func() {
		fmt.Fprintf(GinkgoWriter, "[integration] Start: %s\n", CurrentSpecReport().FullText())

		// start control plane
		var errCP error
		var err error
		var controlPlaneDB *os.File
		controlPlaneDB, err = os.CreateTemp("", "controlplane-*.db")
		Expect(err).NotTo(HaveOccurred())
		controlPlaneDBPath = controlPlaneDB.Name()
		Expect(controlPlaneDB.Close()).To(Succeed())

		controlPlaneCmd := exec.Command(controlPlanePath, "--config", "./testdata/control-plane-config.yaml")
		controlPlaneCmd.Env = append(os.Environ(), "DATABASE_FILEPATH="+controlPlaneDBPath)
		controlPlaneSession, errCP = gexec.Start(
			controlPlaneCmd,
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errCP).NotTo(HaveOccurred())

		// start a SLIM node
		var errNode error
		slimNodeSession, errNode = gexec.Start(
			exec.Command(slimPath, "--config", "./testdata/server-a-config-cp.yaml"),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errNode).NotTo(HaveOccurred())

		// wait for services to start
		time.Sleep(2000 * time.Millisecond)

		// start moderator
		var errModerator error
		moderatorSession, errModerator = gexec.Start(
			exec.Command(clientPath, "--config", "./testdata/moderator-config.yaml", "--local-name", "org/default/moderator1", "--secret", "group-abcdef-12345678901234567890"),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errModerator).NotTo(HaveOccurred())

		// wait for moderator to connect
		time.Sleep(1000 * time.Millisecond)
	})

	AfterEach(func() {
		fmt.Fprintf(GinkgoWriter, "[integration] End: %s\n", CurrentSpecReport().FullText())

		// terminate moderator
		if moderatorSession != nil {
			moderatorSession.Terminate().Wait(30 * time.Second)
		}

		// terminate SLIM node plane
		if slimNodeSession != nil {
			slimNodeSession.Terminate().Wait(30 * time.Second)
		}

		// terminate control plane
		if controlPlaneSession != nil {
			controlPlaneSession.Terminate().Wait(30 * time.Second)
		}

		// delete control plane database file
		if controlPlaneDBPath != "" {
			err := os.Remove(controlPlaneDBPath)
			Expect(err).NotTo(HaveOccurred())
			controlPlaneDBPath = ""
		}
	})

	Describe("group management with control plane with timeout", func() {
		var channelName string
		It("SLIM node creates channel, adds nonexistent participant", func() {
			addChannelOutput, err := exec.Command(
				slimctlPath,
				"c", "channel", "create",
				"moderators=org/default/moderator1/0",
				"-s", "127.0.0.1:50051",
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

			// Invite clientA to the channel
			addClientAOutput, errA := exec.Command(
				slimctlPath,
				"c", "participant", "add",
				participantA,
				"--channel-id", channelName,
				"-s", "127.0.0.1:50051",
			).CombinedOutput()

			time.Sleep(2000 * time.Millisecond)

			Expect(errA).To(HaveOccurred())
			Expect(string(addClientAOutput)).To(ContainSubstring("failed to add participants: unsuccessful response"))
			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say(MsgParticipantAddRequest))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(fmt.Sprintf(MsgParticipantAdded, false)))
		})
	})
})
