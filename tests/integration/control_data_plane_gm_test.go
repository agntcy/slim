// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
	"os/exec"
	"time"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	"github.com/onsi/gomega/gexec"
)

var _ = Describe("Group management through control plane", func() {
	BeforeEach(func() {
		// start control plane
		var errCP error
		controlPlaneSession, errCP = gexec.Start(
			exec.Command(controlPlanePath),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errCP).NotTo(HaveOccurred())

		// start a SLIM node
		// start moderator
		// set up routes
	})

	AfterEach(func() {
		// terminate control plane
		if controlPlaneSession != nil {
			controlPlaneSession.Terminate().Wait(30 * time.Second)
		}
	})
})
