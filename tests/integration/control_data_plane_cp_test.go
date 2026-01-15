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

var _ = Describe("Routing", func() {

	var (
		clientASession *gexec.Session
		clientBSession *gexec.Session
		clientCSession *gexec.Session
	)

	BeforeEach(func() {
	})

	AfterEach(func() {
		// terminate apps
		if clientASession != nil {
			clientASession.Terminate().Wait(2 * time.Second)
		}
		if clientBSession != nil {
			clientBSession.Terminate().Wait(2 * time.Second)
		}
		if clientCSession != nil {
			clientCSession.Terminate().Wait(2 * time.Second)
		}
		// terminate SLIM instances
		if serverASession != nil {
			serverASession.Terminate().Wait(30 * time.Second)
		}
		if serverBSession != nil {
			serverBSession.Terminate().Wait(30 * time.Second)
		}
		// terminate control plane
		if controlPlaneSession != nil {
			controlPlaneSession.Terminate().Wait(30 * time.Second)
		}
		// delete control plane database file
		err := exec.Command("rm", "-f", "controlplane.db").Run()
		Expect(err).NotTo(HaveOccurred())
	})

	Describe("message routing with control plane", func() {
		It("should deliver at least one message each way", func() {
			// start SLIMs
			var errB error

			serverBSession, errB = gexec.Start(
				exec.Command(slimPath, "--config", "./testdata/server-b-config-cp.yaml"),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(errB).NotTo(HaveOccurred())
			time.Sleep(8000 * time.Millisecond)

			var errCP error
			controlPlaneSession, errCP = gexec.Start(
				exec.Command(controlPlanePath, "--config", "./testdata/control-plane-config.yaml"),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(errCP).NotTo(HaveOccurred())
			Eventually(serverBSession.Out, 15*time.Second).Should(gbytes.Say(`connected to control plane`))

			var errA error
			serverASession, errA = gexec.Start(
				exec.Command(slimPath, "--config", "./testdata/server-a-config-cp.yaml"),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(errA).NotTo(HaveOccurred())

			// wait for SLIM instances to start
			time.Sleep(2000 * time.Millisecond)

			var err error
			clientBSession, err = gexec.Start(
				exec.Command(sdkMockPath,
					"--config", "./testdata/client-b-config.yaml",
					"--local-name", "b",
					"--remote-name", "a",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())

			clientCSession, err = gexec.Start(
				exec.Command(sdkMockPath,
					"--config", "./testdata/client-c-config.yaml",
					"--local-name", "c",
					"--remote-name", "a",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())

			time.Sleep(3000 * time.Millisecond)

			clientASession, err = gexec.Start(
				exec.Command(sdkMockPath,
					"--config", "./testdata/client-a-config.yaml",
					"--local-name", "a",
					"--remote-name", "b",
					"--message", "hey",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())

			Eventually(clientBSession.Out, 5*time.Second).
				Should(gbytes.Say(`hello from the a`))

			Eventually(clientASession.Out, 5*time.Second).
				Should(gbytes.Say(`hello from the b`))

			// test listing routes for node a
			routeListOutA, err := exec.Command(
				slimctlPath,
				"controller", "route", "list",
				"-s", "127.0.0.1:50051",
				"-n", "slim/a",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl route list failed: %s", string(routeListOutA))

			routeListOutputA := string(routeListOutA)
			Expect(routeListOutputA).To(ContainSubstring("org/default/b"))
			Expect(routeListOutputA).To(ContainSubstring("org/default/c"))

			// test listing connections for node a
			connectionListOutA, err := exec.Command(
				slimctlPath,
				"controller", "connection", "list",
				"-s", "127.0.0.1:50051",
				"-n", "slim/a",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl connection list failed: %s", string(connectionListOutA))

			connectionOutputA := string(connectionListOutA)
			Expect(connectionOutputA).To(ContainSubstring(":46367"))

			// test listing routes for node b
			routeListOutB, err := exec.Command(
				slimctlPath,
				"controller", "route", "list",
				"-s", "127.0.0.1:50051",
				"-n", "slim/b",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl route list failed: %s", string(routeListOutB))

			routeListOutputB := string(routeListOutB)
			Expect(routeListOutputB).To(ContainSubstring("org/default/a"))

			// test listing connections for node a
			connectionListOutB, err := exec.Command(
				slimctlPath,
				"controller", "connection", "list",
				"-s", "127.0.0.1:50051",
				"-n", "slim/b",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl connection list failed: %s", string(connectionListOutB))

			connectionOutputB := string(connectionListOutB)
			Expect(connectionOutputB).To(ContainSubstring(":46357"))

			clientBSession.Terminate().Wait(2 * time.Second)
			clientCSession.Terminate().Wait(2 * time.Second)

			Eventually(serverBSession.Out, 15*time.Second).Should(gbytes.Say(`notify control plane about lost subscription`))

			time.Sleep(6 * time.Second)

			// test listing routes for node a
			routeListOutA, err = exec.Command(
				slimctlPath,
				"controller", "route", "list",
				"-s", "127.0.0.1:50051",
				"-n", "slim/a",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl route list failed: %s", string(routeListOutA))

			routeListOutputA = string(routeListOutA)

			// test listing routes for node b
			routeListOutB, err = exec.Command(
				slimctlPath,
				"controller", "route", "list",
				"-s", "127.0.0.1:50051",
				"-n", "slim/b",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl route list failed: %s", string(routeListOutB))

			routeListOutputB = string(routeListOutB)
			Expect(routeListOutputB).To(ContainSubstring("org/default/a"))
			Expect(routeListOutputB).ToNot(ContainSubstring("org/default/b"))
			Expect(routeListOutputB).ToNot(ContainSubstring("org/default/c"))
		})
	})

})
