// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
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
		clientBSession      *gexec.Session
	)

	BeforeEach(func() {
		// start control plane
		var errCP error
		controlPlaneSession, errCP = gexec.Start(
			exec.Command(controlPlanePath),
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
			exec.Command(moderatorPath, "--config", "./testdata/moderator-config.yaml", "--name", "moderator1"),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errModerator).NotTo(HaveOccurred())

		// wait for moderator to connect
		time.Sleep(1000 * time.Millisecond)

		// start mock clients
		var errClientA, errClientB error
		clientASession, errClientA = gexec.Start(
			exec.Command(sdkMockPath,
				"--config", "./testdata/client-a-config.yaml",
				"--local-name", "a",
				"--remote-name", "c",
			),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errClientA).NotTo(HaveOccurred())

		clientBSession, errClientB = gexec.Start(
			exec.Command(sdkMockPath,
				"--config", "./testdata/client-c-config.yaml",
				"--local-name", "c",
				"--remote-name", "a",
			),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errClientB).NotTo(HaveOccurred())

		// wait for clients to connect
		time.Sleep(1000 * time.Millisecond)
	})

	AfterEach(func() {
		// terminate clients
		if clientASession != nil {
			clientASession.Terminate().Wait(2 * time.Second)
		}
		if clientBSession != nil {
			clientBSession.Terminate().Wait(2 * time.Second)
		}

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
	})

	Describe("group management with control plane", func() {
		var channelID string
		It("SLIM node creates channel, adds participant, removes participant and deletes channel", func() {
			addChannelOutput, err := exec.Command(
				slimctlPath,
				"channel", "create",
				"moderators=org/default/moderator1/0,org/default/moderator2/0",
				"-s", "127.0.0.1:50051",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred())
			Expect(addChannelOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say("received a create channel request, this should happen"))
			Eventually(moderatorSession, 15*time.Second).Should(gbytes.Say("Controller requested channel creation for channel_id:"))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Channel created successfully"))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Channel saved successfully"))

			// CombinedOutput is "Received response org/default/moderator1/0-... \n"
			// Split channelID by spaces and get the chunk starting with org/default/moderator1/0
			for _, id := range strings.Fields(string(addChannelOutput)) {
				if strings.HasPrefix(id, "org/default/moderator1/0-") {
					channelID = id
					break
				}
			}

			// Invite clientA to the channel
			addClientAOutput, errA := exec.Command(
				slimctlPath,
				"participant", "add",
				"a",
				"--channel-id", channelID,
				"-s", "127.0.0.1:50051",
			).CombinedOutput()

			Expect(errA).NotTo(HaveOccurred())
			Expect(addClientAOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say("received a participant add request for channel"))
			Eventually(moderatorSession, 15*time.Second).Should(gbytes.Say("Controller requested to add participant a to channel"))
			Eventually(moderatorSession, 15*time.Second).Should(gbytes.Say("Successfully invited participant a to channel"))
			// TODO: Verify client receives invitation
			// Eventually(clientASession, 15*time.Second).Should(gbytes.Say("received invitation"))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Ack message received, participant added successfully."))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Channel updated, participant added successfully."))

			// Invite clientB to the channel
			addClientBOutput, errB := exec.Command(
				slimctlPath,
				"participant", "add",
				"c",
				"--channel-id", channelID,
				"-s", "127.0.0.1:50051",
			).CombinedOutput()

			Expect(errB).NotTo(HaveOccurred())
			Expect(addClientBOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say("received a participant add request for channel"))
			Eventually(moderatorSession, 15*time.Second).Should(gbytes.Say("Controller requested to add participant c to channel"))
			Eventually(moderatorSession, 15*time.Second).Should(gbytes.Say("Successfully invited participant c to channel"))
			// TODO: Verify client receives invitation
			// Eventually(clientBSession, 15*time.Second).Should(gbytes.Say("received invitation"))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Ack message received, participant added successfully."))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Channel updated, participant added successfully."))

			// Remove participant c from the channel
			participantID := "c"
			deleteParticipantOutput, errP := exec.Command(
				slimctlPath,
				"participant", "delete",
				participantID,
				"--channel-id", channelID,
				"-s", "127.0.0.1:50051",
			).CombinedOutput()

			Expect(errP).NotTo(HaveOccurred())
			Expect(deleteParticipantOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say("received a participant delete request, this should happen"))

			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Ack message received, participant deleted successfully."))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Channel updated, participant deleted successfully"))

			deleteChannelOutput, errC := exec.Command(
				slimctlPath,
				"channel", "delete",
				channelID,
				"-s", "127.0.0.1:50051",
			).CombinedOutput()

			Expect(errC).NotTo(HaveOccurred())
			Expect(deleteChannelOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say("received a channel delete request, this should happen"))

			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Ack message received, channel deleted successfully."))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Channel deleted successfully"))
		})
	})
})
