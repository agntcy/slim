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
		clientCSession      *gexec.Session
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
			exec.Command(clientPath, "--config", "./testdata/moderator-config.yaml", "--local-name", "org/default/moderator1", "--secret", "group"),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errModerator).NotTo(HaveOccurred())

		// wait for moderator to connect
		time.Sleep(1000 * time.Millisecond)

		// start clients
		var errClientA, errClientC error
		clientASession, errClientA = gexec.Start(
			exec.Command(clientPath,
				"--config", "./testdata/client-a-config.yaml",
				"--local-name", "org/default/a", "--secret", "group",
			),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errClientA).NotTo(HaveOccurred())

		clientCSession, errClientC = gexec.Start(
			exec.Command(clientPath,
				"--config", "./testdata/client-c-config.yaml",
				"--local-name", "org/default/c",
				"--secret", "group",
				"--message", "hey there, I am c!",
			),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errClientC).NotTo(HaveOccurred())

		// wait for clients to connect
		time.Sleep(1000 * time.Millisecond)
	})

	AfterEach(func() {
		// terminate clients
		if clientASession != nil {
			clientASession.Terminate().Wait(2 * time.Second)
		}
		if clientCSession != nil {
			clientCSession.Terminate().Wait(2 * time.Second)
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
		var channelName string
		It("SLIM node creates channel, adds participant, removes participant and deletes channel", func() {
			addChannelOutput, err := exec.Command(
				slimctlPath,
				"channel", "create",
				"moderators=org/default/moderator1/0",
				"-s", "127.0.0.1:50051",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred())
			Expect(addChannelOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say("received a create channel request"))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Channel created successfully"))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Channel saved successfully"))

			// CombinedOutput is "Received response org/default/moderator1/0-... \n"
			// Split channelID by spaces and get the chunk starting with org/default/moderator1/0
			for _, name := range strings.Fields(string(addChannelOutput)) {
				if strings.HasPrefix(name, "org/default") {
					channelName = name
					break
				}
			}

			// Invite clientA to the channel
			addClientAOutput, errA := exec.Command(
				slimctlPath,
				"participant", "add",
				"org/default/a",
				"--channel-id", channelName,
				"-s", "127.0.0.1:50051",
			).CombinedOutput()

			Expect(errA).NotTo(HaveOccurred())
			Expect(addClientAOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say("received a participant add request"))
			Eventually(clientASession, 15*time.Second).Should(gbytes.Say("Session handler task started"))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Ack message received, participant added successfully."))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Channel updated, participant added successfully."))

			// Invite clientC to the channel
			addClientCOutput, errB := exec.Command(
				slimctlPath,
				"participant", "add",
				"org/default/c",
				"--channel-id", channelName,
				"-s", "127.0.0.1:50051",
			).CombinedOutput()

			Expect(errB).NotTo(HaveOccurred())
			Expect(addClientCOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say("received a participant add request"))
			Eventually(clientCSession, 15*time.Second).Should(gbytes.Say("Session handler task started"))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Ack message received, participant added successfully."))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Channel updated, participant added successfully."))

			// Test communication between clientA and clientC
			Eventually(moderatorSession.Out, 10*time.Second).
				Should(gbytes.Say(`hey there, I am c!`))

			Eventually(clientASession.Out, 10*time.Second).
				Should(gbytes.Say(`hey there, I am c!`))

			// // Remove participant c from the channel
			// participantID := "c"
			// deleteParticipantOutput, errP := exec.Command(
			// 	slimctlPath,
			// 	"participant", "delete",
			// 	participantID,
			// 	"--channel-id", channelID,
			// 	"-s", "127.0.0.1:50051",
			// ).CombinedOutput()

			// Expect(errP).NotTo(HaveOccurred())
			// Expect(deleteParticipantOutput).NotTo(BeEmpty())

			// Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say("received a participant delete request"))

			// Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Ack message received, participant deleted successfully."))
			// Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Channel updated, participant deleted successfully"))

			// deleteChannelOutput, errC := exec.Command(
			// 	slimctlPath,
			// 	"channel", "delete",
			// 	channelID,
			// 	"-s", "127.0.0.1:50051",
			// ).CombinedOutput()

			// Expect(errC).NotTo(HaveOccurred())
			// Expect(deleteChannelOutput).NotTo(BeEmpty())

			// Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say("received a channel delete request"))

			// Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Ack message received, channel deleted successfully."))
			// Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say("Channel deleted successfully"))
		})
	})
})
