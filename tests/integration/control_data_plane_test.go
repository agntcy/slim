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
		clientASession *gexec.Session
		clientBSession *gexec.Session
		serverASession *gexec.Session
		serverBSession *gexec.Session

		tempDir          string
		serverAConfig    string
		serverBConfig    string
		clientAConfig    string
		clientBConfig    string
		clientAConfigVia string
		clientBConfigVia string

		dataPlaneBPort  int
		controllerAPort int
		controllerBPort int
	)

	BeforeEach(func() {
		dataPlaneAPort := reservePort()
		dataPlaneBPort = reservePort()
		controllerAPort = reservePort()
		controllerBPort = reservePort()

		replacementsA := map[string]string{
			"0.0.0.0:46357":          fmt.Sprintf("0.0.0.0:%d", dataPlaneAPort),
			"0.0.0.0:46358":          fmt.Sprintf("0.0.0.0:%d", controllerAPort),
			"http://localhost:46357": fmt.Sprintf("http://localhost:%d", dataPlaneAPort),
			"http://127.0.0.1:46357": fmt.Sprintf("http://127.0.0.1:%d", dataPlaneAPort),
		}
		replacementsB := map[string]string{
			"0.0.0.0:46357":          fmt.Sprintf("0.0.0.0:%d", dataPlaneBPort),
			"0.0.0.0:46358":          fmt.Sprintf("0.0.0.0:%d", controllerBPort),
			"http://localhost:46357": fmt.Sprintf("http://localhost:%d", dataPlaneBPort),
			"http://127.0.0.1:46357": fmt.Sprintf("http://127.0.0.1:%d", dataPlaneBPort),
		}
		clientReplacementsA := map[string]string{
			"http://localhost:46357": fmt.Sprintf("http://localhost:%d", dataPlaneAPort),
			"http://127.0.0.1:46357": fmt.Sprintf("http://127.0.0.1:%d", dataPlaneAPort),
		}
		clientReplacementsB := map[string]string{
			"http://localhost:46357": fmt.Sprintf("http://localhost:%d", dataPlaneBPort),
			"http://127.0.0.1:46357": fmt.Sprintf("http://127.0.0.1:%d", dataPlaneBPort),
		}
		jsonReplacements := map[string]string{
			"http://127.0.0.1:46357": fmt.Sprintf("http://127.0.0.1:%d", dataPlaneAPort),
			"http://127.0.0.1:46367": fmt.Sprintf("http://127.0.0.1:%d", dataPlaneBPort),
		}

		tempDir = newTempDir("slim-integration-routing-")
		serverAConfig = writeTempConfig(tempDir, "./testdata/server-with-controller.yaml", "server-a-config.yaml", replacementsA)
		serverBConfig = writeTempConfig(tempDir, "./testdata/server-with-controller.yaml", "server-b-config.yaml", replacementsB)
		clientAConfig = writeTempConfig(tempDir, "./testdata/client.yaml", "client-a-config.yaml", clientReplacementsA)
		clientBConfig = writeTempConfig(tempDir, "./testdata/client.yaml", "client-b-config.yaml", clientReplacementsB)
		clientAConfigVia = writeTempConfig(tempDir, "./testdata/client-a-config-data.json", "client-a-config-data.json", jsonReplacements)
		clientBConfigVia = writeTempConfig(tempDir, "./testdata/client-b-config-data.json", "client-b-config-data.json", jsonReplacements)

		// start SLIMs
		var errA, errB error
		serverASession, errA = gexec.Start(
			exec.Command(slimPath, "--config", serverAConfig),
			GinkgoWriter, GinkgoWriter,
		)
		serverBSession, errB = gexec.Start(
			exec.Command(slimPath, "--config", serverBConfig),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(errA).NotTo(HaveOccurred())
		Expect(errB).NotTo(HaveOccurred())

		// wait for SLIM instances to start
		time.Sleep(2000 * time.Millisecond)
		Eventually(serverASession.Out, 15*time.Second).Should(gbytes.Say("started controlplane server"))
		Eventually(serverBSession.Out, 15*time.Second).Should(gbytes.Say("started controlplane server"))

		// add routes
		runCombinedOutputWithRetry(10*time.Second, func() *exec.Cmd {
			return exec.Command(slimctlPath, "n",
				"route", "add", "org/default/b/0",
				"via", clientBConfigVia,
				"-s", fmt.Sprintf("127.0.0.1:%d", controllerAPort),
			)
		})

		runCombinedOutputWithRetry(10*time.Second, func() *exec.Cmd {
			return exec.Command(slimctlPath, "n",
				"route", "add", "org/default/a/0",
				"via", clientAConfigVia,
				"-s", fmt.Sprintf("127.0.0.1:%d", controllerBPort),
			)
		})
	})

	AfterEach(func() {
		// terminate apps
		terminateSession(clientASession, 2*time.Second)
		terminateSession(clientBSession, 2*time.Second)
		// terminate SLIM instances
		terminateSession(serverASession, 30*time.Second)
		terminateSession(serverBSession, 30*time.Second)

		if tempDir != "" {
			_ = os.RemoveAll(tempDir)
			tempDir = ""
		}
	})

	Describe("message routing", func() {
		It("should deliver at least one message each way", func() {
			var err error

			clientBSession, err = gexec.Start(
				exec.Command(sdkMockPath,
					"--config", clientBConfig,
					"--local-name", "b",
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
		})

		It("should have the valid routes and connections", func() {
			// test listing routes
			routeListOut, err := exec.Command(
				slimctlPath, "n",
				"route", "list",
				"-s", fmt.Sprintf("127.0.0.1:%d", controllerAPort),
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl route list failed: %s", string(routeListOut))

			routeListOutput := string(routeListOut)
			Expect(routeListOutput).To(ContainSubstring("org/default/b id=0"))

			// test listing connections
			connectionListOut, err := exec.Command(
				slimctlPath, "n",
				"connection", "list",
				"-s", fmt.Sprintf("127.0.0.1:%d", controllerAPort),
			).CombinedOutput()
			Expect(err).NotTo(HaveOccurred(), "slimctl connection list failed: %s", string(connectionListOut))

			connectionOutput := string(connectionListOut)
			Expect(connectionOutput).To(ContainSubstring(fmt.Sprintf(":%d", dataPlaneBPort)))
		})
	})
})
