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

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say(MsgCreateChannelRequest))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(MsgChannelCreatedSuccessfully))
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
				"participant", "add",
				participantA,
				"--channel-id", channelName,
				"-s", "127.0.0.1:50051",
			).CombinedOutput()

			Expect(errA).NotTo(HaveOccurred())
			Expect(addClientAOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say(MsgParticipantAddRequest))
			Eventually(clientASession, 15*time.Second).Should(gbytes.Say(MsgSessionHandlerTaskStarted))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(MsgAckParticipantAddedSuccessfully))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(MsgChannelUpdatedParticipantAdded))

			// Invite clientC to the channel
			addClientCOutput, errB := exec.Command(
				slimctlPath,
				"participant", "add",
				participantC,
				"--channel-id", channelName,
				"-s", "127.0.0.1:50051",
			).CombinedOutput()

			Expect(errB).NotTo(HaveOccurred())
			Expect(addClientCOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say(MsgParticipantAddRequest))
			Eventually(clientCSession, 15*time.Second).Should(gbytes.Say(MsgSessionHandlerTaskStarted))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(MsgAckParticipantAddedSuccessfully))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(MsgChannelUpdatedParticipantAdded))

			// Test communication between clientA and clientC
			Eventually(moderatorSession.Out, 10*time.Second).
				Should(gbytes.Say(MsgTestClientCMessage))

			Eventually(clientASession.Out, 10*time.Second).
				Should(gbytes.Say(MsgTestClientCMessage))

			// Remove participant c from the channel
			deleteParticipantOutput, errP := exec.Command(
				slimctlPath,
				"participant", "delete",
				participantC,
				"--channel-id", channelName,
				"-s", "127.0.0.1:50051",
			).CombinedOutput()

			Expect(errP).NotTo(HaveOccurred())
			Expect(deleteParticipantOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say(MsgParticipantDeleteRequest))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(MsgAckParticipantDeletedSuccessfully))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(MsgChannelUpdatedParticipantDeleted))
			Eventually(clientCSession, 15*time.Second).Should(gbytes.Say(MsgSessionClosed))

			deleteChannelOutput, errC := exec.Command(
				slimctlPath,
				"channel", "delete",
				channelName,
				"-s", "127.0.0.1:50051",
			).CombinedOutput()

			Expect(errC).NotTo(HaveOccurred())
			Expect(deleteChannelOutput).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say(MsgChannelDeleteRequest))

			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(MsgAckChannelDeletedSuccessfully))
			Eventually(controlPlaneSession, 15*time.Second).Should(gbytes.Say(MsgChannelDeletedSuccessfully))
			Eventually(clientASession, 15*time.Second).Should(gbytes.Say(MsgSessionClosed))
			Eventually(moderatorSession, 15*time.Second).Should(gbytes.Say(MsgSessionClosed))
		})
	})
})
