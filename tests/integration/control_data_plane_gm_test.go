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

		// set up routes
	})

	AfterEach(func() {
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
		It("SLIM node receives the 'create group API call'", func() {
			output, err := exec.Command(
				slimctlPath,
				"channel", "create",
				"moderators=moderator1,moderator2",
				"-s", "127.0.0.1:50051",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred())
			Expect(output).NotTo(BeEmpty())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say("received a create channel request, this should happen"))
			Eventually(moderatorSession, 15*time.Second).Should(gbytes.Say("Controller requested channel creation for channel_id:"))

			// CombinedOutput is "Received response moderator1-... \n"
			// Split channelID by spaces and get the chunk starting with moderator1
			channelID := ""
			for _, id := range strings.Fields(string(output)) {
				if strings.HasPrefix(id, "moderator1-") {
					channelID = id
					break
				}
			}

			_, errP := exec.Command(
				slimctlPath,
				"participant", "add",
				"participant1",
				"--channel-id", channelID,
				"-s", "127.0.0.1:50051",
			).CombinedOutput()

			Expect(errP).NotTo(HaveOccurred())

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say("received a participant add request, this should happen"))

		})
	})
})
