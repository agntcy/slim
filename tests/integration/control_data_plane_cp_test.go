// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
	"fmt"
	"os"
	"os/exec"
	"time"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	"github.com/onsi/gomega/gbytes"
	"github.com/onsi/gomega/gexec"
)

var _ = Describe("Routing", func() {

	var (
		clientASession      *gexec.Session
		clientBSession      *gexec.Session
		clientCSession      *gexec.Session
		serverASession      *gexec.Session
		serverBSession      *gexec.Session
		controlPlaneSession *gexec.Session
		controlPlaneDBPath  string

		tempDir               string
		serverAConfig         string
		serverBConfig         string
		clientAConfig         string
		clientBConfig         string
		controlPlaneConfig    string
		controlPlaneNorthPort int
		controlPlaneSouthPort int
		dataPlaneAPort        int
		dataPlaneBPort        int
	)

	BeforeEach(func() {
		fmt.Fprintf(GinkgoWriter, "[integration] Start: %s\n", CurrentSpecReport().FullText())

		dataPlaneAPort = reservePort()
		dataPlaneBPort = reservePort()
		controlPlaneNorthPort = reservePort()
		controlPlaneSouthPort = reservePort()
		controllerBPort := reservePort()

		replacements := map[string]string{
			"0.0.0.0:46357":          fmt.Sprintf("0.0.0.0:%d", dataPlaneAPort),
			"0.0.0.0:46367":          fmt.Sprintf("0.0.0.0:%d", dataPlaneBPort),
			"0.0.0.0:46368":          fmt.Sprintf("0.0.0.0:%d", controllerBPort),
			"http://localhost:46357": fmt.Sprintf("http://localhost:%d", dataPlaneAPort),
			"http://localhost:46367": fmt.Sprintf("http://localhost:%d", dataPlaneBPort),
			"http://127.0.0.1:46357": fmt.Sprintf("http://127.0.0.1:%d", dataPlaneAPort),
			"http://127.0.0.1:46367": fmt.Sprintf("http://127.0.0.1:%d", dataPlaneBPort),
			"http://127.0.0.1:50051": fmt.Sprintf("http://127.0.0.1:%d", controlPlaneNorthPort),
			"http://127.0.0.1:50052": fmt.Sprintf("http://127.0.0.1:%d", controlPlaneSouthPort),
			"httpPort: 50051":        fmt.Sprintf("httpPort: %d", controlPlaneNorthPort),
			"httpPort: 50052":        fmt.Sprintf("httpPort: %d", controlPlaneSouthPort),
		}

		tempDir = newTempDir("slim-integration-control-plane-")
		serverAConfig = writeTempConfig(tempDir, "./testdata/server-a-config-cp.yaml", "server-a-config-cp.yaml", replacements)
		serverBConfig = writeTempConfig(tempDir, "./testdata/server-b-config-cp.yaml", "server-b-config-cp.yaml", replacements)
		clientAConfig = writeTempConfig(tempDir, "./testdata/client-a-config.yaml", "client-a-config.yaml", replacements)
		clientBConfig = writeTempConfig(tempDir, "./testdata/client-b-config.yaml", "client-b-config.yaml", replacements)
		controlPlaneConfig = writeTempConfig(tempDir, "./testdata/control-plane-config.yaml", "control-plane-config.yaml", replacements)
	})

	AfterEach(func() {
		fmt.Fprintf(GinkgoWriter, "[integration] End: %s\n", CurrentSpecReport().FullText())
		// terminate apps
		terminateSession(clientASession, 2*time.Second)
		terminateSession(clientBSession, 2*time.Second)
		terminateSession(clientCSession, 2*time.Second)
		// terminate SLIM instances
		terminateSession(serverASession, 30*time.Second)
		terminateSession(serverBSession, 30*time.Second)
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

	Describe("message routing with control plane", func() {
		It("should deliver at least one message each way", func() {
			// start SLIM node b
			var errB error

			serverBSession, errB = gexec.Start(
				exec.Command(slimPath, "--config", serverBConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(errB).NotTo(HaveOccurred())
			time.Sleep(8000 * time.Millisecond)

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
			// test if SLIM node b connects to control plane
			Eventually(serverBSession.Out, 15*time.Second).Should(gbytes.Say(`connected to control plane`))

			// start SLIM node a
			var errA error
			serverASession, errA = gexec.Start(
				exec.Command(slimPath, "--config", serverAConfig),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(errA).NotTo(HaveOccurred())

			// wait for SLIM node a to start
			time.Sleep(2000 * time.Millisecond)

			// test if SLIM node a connects to control plane
			Eventually(serverASession.Out, 15*time.Second).Should(gbytes.Say(`connected to control plane`))

			clientBSession, err = gexec.Start(
				exec.Command(sdkMockPath,
					"--config", clientBConfig,
					"--local-name", "b1",
					"--remote-name", "a",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())

			clientCSession, err = gexec.Start(
				exec.Command(sdkMockPath,
					"--config", clientBConfig,
					"--local-name", "b2",
					"--remote-name", "a",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())

			time.Sleep(3000 * time.Millisecond)

			clientASession, err = gexec.Start(
				exec.Command(sdkMockPath,
					"--config", clientAConfig,
					"--local-name", "a",
					"--remote-name", "b1",
					"--message", "hey",
				),
				GinkgoWriter, GinkgoWriter,
			)
			Expect(err).NotTo(HaveOccurred())

			Eventually(clientBSession.Out, 5*time.Second).
				Should(gbytes.Say(`hello from the a`))

			Eventually(clientASession.Out, 5*time.Second).
				Should(gbytes.Say(`hello from the b1`))

			// test listing routes for node a
			routeListOutA, err := exec.Command(
				slimctlPath,
				"-s", fmt.Sprintf("127.0.0.1:%d", controlPlaneNorthPort), "--tls-insecure",
				"controller", "route", "list",
				"-n", "slim/a",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl route list failed: %s", string(routeListOutA))

			routeListOutputA := string(routeListOutA)
			Expect(routeListOutputA).To(ContainSubstring("org/default/b1"))
			Expect(routeListOutputA).To(ContainSubstring("org/default/b2"))

			// test listing connections for node a
			connectionListOutA, err := exec.Command(
				slimctlPath,
				"-s", fmt.Sprintf("127.0.0.1:%d", controlPlaneNorthPort), "--tls-insecure",
				"controller", "connection", "list",
				"-n", "slim/a",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl connection list failed: %s", string(connectionListOutA))

			connectionOutputA := string(connectionListOutA)
			Expect(connectionOutputA).To(ContainSubstring(fmt.Sprintf(":%d", dataPlaneBPort)))

			// test listing routes for node b
			routeListOutB, err := exec.Command(
				slimctlPath,
				"-s", fmt.Sprintf("127.0.0.1:%d", controlPlaneNorthPort), "--tls-insecure",
				"controller", "route", "list",
				"-n", "slim/b",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl route list failed: %s", string(routeListOutB))

			routeListOutputB := string(routeListOutB)
			Expect(routeListOutputB).To(ContainSubstring("org/default/a"))

			// test listing connections for node b
			connectionListOutB, err := exec.Command(
				slimctlPath,
				"-s", fmt.Sprintf("127.0.0.1:%d", controlPlaneNorthPort), "--tls-insecure",
				"controller", "connection", "list",
				"-n", "slim/b",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl connection list failed: %s", string(connectionListOutB))

			connectionOutputB := string(connectionListOutB)
			Expect(connectionOutputB).To(ContainSubstring(fmt.Sprintf(":%d", dataPlaneAPort)))

			terminateSession(clientBSession, 2*time.Second)
			terminateSession(clientCSession, 2*time.Second)

			Eventually(serverBSession.Out, 15*time.Second).Should(gbytes.Say(`notify control plane about lost subscription`))

			time.Sleep(6 * time.Second)

			// test listing routes for node a
			routeListOutA, err = exec.Command(
				slimctlPath,
				"-s", fmt.Sprintf("127.0.0.1:%d", controlPlaneNorthPort), "--tls-insecure",
				"controller", "route", "list",
				"-n", "slim/a",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl route list failed: %s", string(routeListOutA))

			routeListOutputA = string(routeListOutA)

			// test listing routes for node b
			routeListOutB, err = exec.Command(
				slimctlPath,
				"-s", fmt.Sprintf("127.0.0.1:%d", controlPlaneNorthPort), "--tls-insecure",
				"controller", "route", "list",
				"-n", "slim/b",
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl route list failed: %s", string(routeListOutB))

			routeListOutputB = string(routeListOutB)
			Expect(routeListOutputB).To(ContainSubstring("org/default/a"))
			Expect(routeListOutputB).ToNot(ContainSubstring("org/default/b1"))
			Expect(routeListOutputB).ToNot(ContainSubstring("org/default/b2"))
		})
	})

})
