// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
	"os/exec"
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
		// set up routes
	})

	AfterEach(func() {
		// terminate control plane
		if controlPlaneSession != nil {
			controlPlaneSession.Terminate().Wait(30 * time.Second)
		}

		// terminate SLIM node plane
		if slimNodeSession != nil {
			slimNodeSession.Terminate().Wait(30 * time.Second)
		}
	})

	Describe("group management with control plane", func() {
		It("SLIM node receives the 'create group API call'", func() {
			exec.Command(
				slimctlPath,
				"channel", "create",
				"moderators=moderator1,moderator2",
				"-s", "127.0.0.1:50051",
			).CombinedOutput()

			Eventually(slimNodeSession, 15*time.Second).Should(gbytes.Say("received a create channel request, this should happen"))
		})
	})
})
