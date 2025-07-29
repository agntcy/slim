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
	)

	BeforeEach(func() {
		// start SLIMs
		var errA, errB error
		serverASession, errA = gexec.Start(
			exec.Command(slimPath, "--config", "./testdata/server-a-config.yaml"),
			GinkgoWriter, GinkgoWriter,
		)
		serverBSession, errB = gexec.Start(
			exec.Command(slimPath, "--config", "./testdata/server-b-config.yaml"),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errA).NotTo(HaveOccurred())
		Expect(errB).NotTo(HaveOccurred())

		// wait for SLIM instances to start
		time.Sleep(2000 * time.Millisecond)

		// add routes
		Expect(exec.Command(slimctlPath, "nc",
			"route", "add", "org/default/b/0",
			"via", "./testdata/client-b-config-data.json",
			"-s", "127.0.0.1:46358",
		).Run()).To(Succeed())

		Expect(exec.Command(slimctlPath, "nc",
			"route", "add", "org/default/a/0",
			"via", "./testdata/client-a-config-data.json",
			"-s", "127.0.0.1:46368",
		).Run()).To(Succeed())
	})

	AfterEach(func() {
		// terminate agents
		if clientASession != nil {
			clientASession.Terminate().Wait(2 * time.Second)
		}
		if clientBSession != nil {
			clientBSession.Terminate().Wait(2 * time.Second)
		}
		// terminate SLIM instances
		if serverASession != nil {
			serverASession.Terminate().Wait(30 * time.Second)
		}
		if serverBSession != nil {
			serverBSession.Terminate().Wait(30 * time.Second)
		}
	})

	Describe("message routing", func() {
		It("should deliver at least one message each way", func() {
			var err error

			clientBSession, err = gexec.Start(
				exec.Command(sdkMockPath,
					"--config", "./testdata/client-b-config.yaml",
					"--local-agent", "b",
					"--remote-agent", "a",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())

			time.Sleep(3000 * time.Millisecond)

			clientASession, err = gexec.Start(
				exec.Command(sdkMockPath,
					"--config", "./testdata/client-a-config.yaml",
					"--local-agent", "a",
					"--remote-agent", "b",
					"--message", "hey",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())

			Eventually(clientBSession.Out, 5*time.Second).
				Should(gbytes.Say(`received message: hello from the a`))

			Eventually(clientASession.Out, 5*time.Second).
				Should(gbytes.Say(`received message: hello from the b`))
		})

		It("should have the valid routes and connections", func() {
			// test listing routes
			routeListOut, err := exec.Command(
				slimctlPath, "nc",
				"route", "list",
				"-s", "127.0.0.1:46358",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl route list failed: %s", string(routeListOut))

			routeListOutput := string(routeListOut)
			Expect(routeListOutput).To(ContainSubstring("org/default/b id=0"))

			// test listing connections
			connectionListOut, err := exec.Command(
				slimctlPath, "nc",
				"connection", "list",
				"-s", "127.0.0.1:46358",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl connection list failed: %s", string(connectionListOut))

			connectionOutput := string(connectionListOut)
			Expect(connectionOutput).To(ContainSubstring(":46367"))
		})
	})
})
