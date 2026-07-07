// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

// Integration test for packet header integrity validation between two nodes.
//
// 2 cases are covered:
// - happy path: two nodes are set up with link negotiation (and HMAC generation through ECDH), then remote clients communicate
//
// - tampered packet: one of the nodes tampers the destination field after signing the header HMAC. The federated node will fail
//   the validation and drop the packet.

import (
	"fmt"
	"os"
	"os/exec"
	"regexp"
	"time"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	"github.com/onsi/gomega/gbytes"
	"github.com/onsi/gomega/gexec"
)

// sessionLog returns captured stdout/stderr for a gexec session. Use with ContainSubstring
// instead of gbytes.Say when multiple SLIM nodes run in parallel (Say only matches new bytes
// since the last successful Say on the same buffer).
func sessionLog(session *gexec.Session) string {
	return string(session.Out.Contents()) + string(session.Err.Contents())
}

func headerIntegrityControlPlaneReplacements(nodeAPort, nodeBPort, ctrlAPort, ctrlBPort int) map[string]string {
	return map[string]string{
		"0.0.0.0:46357":          fmt.Sprintf("0.0.0.0:%d", nodeAPort),
		"0.0.0.0:46358":          fmt.Sprintf("0.0.0.0:%d", ctrlAPort),
		"0.0.0.0:46481":          fmt.Sprintf("0.0.0.0:%d", nodeBPort),
		"0.0.0.0:46482":          fmt.Sprintf("0.0.0.0:%d", ctrlBPort),
		"http://localhost:46480": fmt.Sprintf("http://localhost:%d", nodeAPort),
	}
}

var _ = Describe("Header integrity across federated dataplane nodes", func() {
	var tempDir string

	BeforeEach(func() {
		tempDir = newTempDir("slim-integration-header-mac-")
	})

	AfterEach(func() {
		if tempDir != "" {
			_ = os.RemoveAll(tempDir)
			tempDir = ""
		}
	})

	PIt("completes link negotiation, verifies header MAC in dataplane tests, and routes sdk-mock across nodes via control plane", func() { // TODO: uses removed `route add` command; update to topology-based routing
		nodeAPort := reservePort()
		nodeBPort := reservePort()
		controllerAPort := reservePort()
		controllerBPort := reservePort()

		repl := headerIntegrityControlPlaneReplacements(nodeAPort, nodeBPort, controllerAPort, controllerBPort)

		nodeAConfig := writeTempConfig(tempDir, "./testdata/header-mac-node-a.yaml", "node-a.yaml", repl)
		nodeBConfig := writeTempConfig(tempDir, "./testdata/header-mac-node-b.yaml", "node-b.yaml", repl)

		clientReplacementsA := map[string]string{
			"http://localhost:46357": fmt.Sprintf("http://localhost:%d", nodeAPort),
		}
		clientReplacementsB := map[string]string{
			"http://localhost:46357": fmt.Sprintf("http://localhost:%d", nodeBPort),
		}
		clientAConfig := writeTempConfig(tempDir, "./testdata/client.yaml", "header-mac-client-a.yaml", clientReplacementsA)
		clientBConfig := writeTempConfig(tempDir, "./testdata/client.yaml", "header-mac-client-b.yaml", clientReplacementsB)

		viaPeerB := map[string]string{
			"http://127.0.0.1:46481": fmt.Sprintf("http://127.0.0.1:%d", nodeBPort),
		}
		viaPeerA := map[string]string{
			"http://127.0.0.1:46357": fmt.Sprintf("http://127.0.0.1:%d", nodeAPort),
		}
		clientAVia := writeTempConfig(tempDir, "./testdata/header-mac-via-peer-b.json", "header-mac-client-a-via.json", viaPeerB)
		clientBVia := writeTempConfig(tempDir, "./testdata/header-mac-via-peer-a.json", "header-mac-client-b-via.json", viaPeerA)

		nodeASession, err := gexec.Start(
			exec.Command(slimPath, "--config", nodeAConfig),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(err).NotTo(HaveOccurred())
		defer terminateSession(nodeASession, 30*time.Second)

		Eventually(nodeASession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

		nodeBSession, err := gexec.Start(
			exec.Command(slimPath, "--config", nodeBConfig),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(err).NotTo(HaveOccurred())
		defer terminateSession(nodeBSession, 30*time.Second)

		Eventually(nodeBSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))
		Eventually(nodeASession.Out, 15*time.Second).Should(gbytes.Say("started controlplane server"))
		Eventually(nodeBSession.Out, 15*time.Second).Should(gbytes.Say("started controlplane server"))
		Eventually(func() string { return sessionLog(nodeBSession) }).Should(ContainSubstring("client connected"))
		Eventually(func() string { return sessionLog(nodeASession) }).Should(ContainSubstring("received link negotiation"))
		Eventually(func() string { return sessionLog(nodeBSession) }).Should(ContainSubstring("received link negotiation"))

		getRouteWithID := func(controllerPort int, routePrefix string) string {
			var routeName string
			re := regexp.MustCompile(fmt.Sprintf(`(%s/\S+)\s+connections=`, regexp.QuoteMeta(routePrefix)))
			Eventually(func() string {
				out, err := exec.Command(
					slimctlPath, "n",
					"route", "list",
					"--server", fmt.Sprintf("127.0.0.1:%d", controllerPort),
				).CombinedOutput()
				if err != nil {
					return ""
				}
				matches := re.FindStringSubmatch(string(out))
				if len(matches) >= 2 {
					routeName = matches[1]
					return routeName
				}
				return ""
			}, 10*time.Second, 500*time.Millisecond).ShouldNot(BeEmpty(),
				"timed out waiting for route with real app ID for %s", routePrefix)
			return routeName
		}

		var clientASession, clientBSession *gexec.Session

		clientBSession, err = gexec.Start(
			exec.Command(sdkMockPath,
				"--config", clientBConfig,
				"--local-name", "b",
				"--remote-name", "a",
			),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(err).NotTo(HaveOccurred())
		defer terminateSession(clientBSession, 2*time.Second)

		routeB := getRouteWithID(controllerBPort, "org/default/b")
		runCombinedOutputWithRetry(10*time.Second, func() *exec.Cmd {
			return exec.Command(slimctlPath, "n",
				"route", "add", routeB,
				"via", clientAVia,
				"--server", fmt.Sprintf("127.0.0.1:%d", controllerAPort),
			)
		})

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
		defer terminateSession(clientASession, 2*time.Second)

		routeA := getRouteWithID(controllerAPort, "org/default/a")
		runCombinedOutputWithRetry(10*time.Second, func() *exec.Cmd {
			return exec.Command(slimctlPath, "n",
				"route", "add", routeA,
				"via", clientBVia,
				"--server", fmt.Sprintf("127.0.0.1:%d", controllerBPort),
			)
		})

		Eventually(clientBSession.Out, 15*time.Second).Should(gbytes.Say(`hello from the a`))
		Eventually(clientASession.Out, 15*time.Second).Should(gbytes.Say(`hello from the b`))
	})

	PIt("rejects destination-tampered inter-node publishes: peer logs MAC failure and subscriber sees no delivery", func() { // TODO: uses removed `route add` command; update to topology-based routing
		nodeAPort := reservePort()
		nodeBPort := reservePort()
		controllerAPort := reservePort()
		controllerBPort := reservePort()

		repl := headerIntegrityControlPlaneReplacements(nodeAPort, nodeBPort, controllerAPort, controllerBPort)

		nodeAConfig := writeTempConfig(tempDir, "./testdata/header-mac-node-a.yaml", "node-a-tamper.yaml", repl)
		nodeBConfig := writeTempConfig(tempDir, "./testdata/header-mac-node-b.yaml", "node-b-tamper.yaml", repl)

		clientReplacementsA := map[string]string{
			"http://localhost:46357": fmt.Sprintf("http://localhost:%d", nodeAPort),
		}
		clientReplacementsB := map[string]string{
			"http://localhost:46357": fmt.Sprintf("http://localhost:%d", nodeBPort),
		}
		clientAConfig := writeTempConfig(tempDir, "./testdata/client.yaml", "header-mac-tamper-client-a.yaml", clientReplacementsA)
		clientBConfig := writeTempConfig(tempDir, "./testdata/client.yaml", "header-mac-tamper-client-b.yaml", clientReplacementsB)

		viaPeerB := map[string]string{
			"http://127.0.0.1:46481": fmt.Sprintf("http://127.0.0.1:%d", nodeBPort),
		}
		viaPeerA := map[string]string{
			"http://127.0.0.1:46357": fmt.Sprintf("http://127.0.0.1:%d", nodeAPort),
		}
		clientAVia := writeTempConfig(tempDir, "./testdata/header-mac-via-peer-b.json", "header-mac-tamper-client-a-via.json", viaPeerB)
		clientBVia := writeTempConfig(tempDir, "./testdata/header-mac-via-peer-a.json", "header-mac-tamper-client-b-via.json", viaPeerA)

		nodeACmd := exec.Command(slimPath, "--config", nodeAConfig)
		nodeACmd.Env = append(os.Environ(), "SLIM_TEST_TAMPER_DESTINATION=1")
		nodeASession, err := gexec.Start(nodeACmd, GinkgoWriter, GinkgoWriter)
		Expect(err).NotTo(HaveOccurred())
		defer terminateSession(nodeASession, 30*time.Second)

		Eventually(nodeASession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

		nodeBSession, err := gexec.Start(
			exec.Command(slimPath, "--config", nodeBConfig),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(err).NotTo(HaveOccurred())
		defer terminateSession(nodeBSession, 30*time.Second)

		Eventually(nodeBSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))
		Eventually(nodeASession.Out, 15*time.Second).Should(gbytes.Say("started controlplane server"))
		Eventually(nodeBSession.Out, 15*time.Second).Should(gbytes.Say("started controlplane server"))
		Eventually(func() string { return sessionLog(nodeBSession) }).Should(ContainSubstring("client connected"))
		Eventually(func() string { return sessionLog(nodeASession) }).Should(ContainSubstring("received link negotiation"))
		Eventually(func() string { return sessionLog(nodeBSession) }).Should(ContainSubstring("received link negotiation"))

		getRouteWithID := func(controllerPort int, routePrefix string) string {
			var routeName string
			re := regexp.MustCompile(fmt.Sprintf(`(%s/\S+)\s+connections=`, regexp.QuoteMeta(routePrefix)))
			Eventually(func() string {
				out, err := exec.Command(
					slimctlPath, "n",
					"route", "list",
					"--server", fmt.Sprintf("127.0.0.1:%d", controllerPort),
				).CombinedOutput()
				if err != nil {
					return ""
				}
				matches := re.FindStringSubmatch(string(out))
				if len(matches) >= 2 {
					routeName = matches[1]
					return routeName
				}
				return ""
			}, 10*time.Second, 500*time.Millisecond).ShouldNot(BeEmpty(),
				"timed out waiting for route with real app ID for %s", routePrefix)
			return routeName
		}

		var clientASession, clientBSession *gexec.Session

		clientBSession, err = gexec.Start(
			exec.Command(sdkMockPath,
				"--config", clientBConfig,
				"--local-name", "b",
				"--remote-name", "a",
			),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(err).NotTo(HaveOccurred())
		defer terminateSession(clientBSession, 2*time.Second)

		routeB := getRouteWithID(controllerBPort, "org/default/b")
		runCombinedOutputWithRetry(10*time.Second, func() *exec.Cmd {
			return exec.Command(slimctlPath, "n",
				"route", "add", routeB,
				"via", clientAVia,
				"--server", fmt.Sprintf("127.0.0.1:%d", controllerAPort),
			)
		})

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
		defer terminateSession(clientASession, 2*time.Second)

		routeA := getRouteWithID(controllerAPort, "org/default/a")
		runCombinedOutputWithRetry(10*time.Second, func() *exec.Cmd {
			return exec.Command(slimctlPath, "n",
				"route", "add", routeA,
				"via", clientBVia,
				"--server", fmt.Sprintf("127.0.0.1:%d", controllerBPort),
			)
		})

		Eventually(nodeBSession.Out, 20*time.Second).Should(gbytes.Say("SLIM header integrity verification failed"))
		Consistently(clientBSession.Out, 5*time.Second, 250*time.Millisecond).ShouldNot(gbytes.Say("hello from the a"))
		Consistently(clientASession.Out, 5*time.Second, 250*time.Millisecond).ShouldNot(gbytes.Say("hello from the b"))
	})
})
