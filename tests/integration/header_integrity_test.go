// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	"github.com/onsi/gomega/gbytes"
	"github.com/onsi/gomega/gexec"
)

// envWithSlimHeaderTamper returns the current environment with SLIM_TEST_TAMPER_DESTINATION=1,
// replacing any existing entry so the child process always sees the intended value.
func envWithSlimHeaderTamper() []string {
	const prefix = "SLIM_TEST_TAMPER_DESTINATION="
	out := make([]string, 0, len(os.Environ())+1)
	for _, e := range os.Environ() {
		if !strings.HasPrefix(e, prefix) {
			out = append(out, e)
		}
	}
	return append(out, prefix+"1")
}

// injectDataplaneServerHeaderMac adds header_mac_key next to tls on the dataplane listen server only
// (see testdata/server-with-controller.yaml). dataplanePort must match the substituted listen port.
func injectDataplaneServerHeaderMac(yaml string, dataplanePort int, macKey string) string {
	block := fmt.Sprintf(`        - endpoint: "0.0.0.0:%d"
          tls:
            insecure: true`, dataplanePort)
	replacement := fmt.Sprintf(`        - endpoint: "0.0.0.0:%d"
          tls:
            insecure: true
          header_mac_key: "%s"`, dataplanePort, macKey)
	Expect(strings.Contains(yaml, block)).To(BeTrue(), "expected dataplane server block for header_mac injection")
	return strings.Replace(yaml, block, replacement, 1)
}

func writeHeaderIntegrityNodeConfig(
	dir, fileName string,
	serviceRenames map[string]string,
	nodeDPPort, nodeCPPort, peerDPPort int,
	macKey string,
) string {
	contents, err := os.ReadFile("./testdata/server-with-controller.yaml")
	Expect(err).NotTo(HaveOccurred())
	s := string(contents)
	for old, newVal := range serviceRenames {
		s = strings.ReplaceAll(s, old, newVal)
	}
	s = strings.ReplaceAll(s, "0.0.0.0:46357", fmt.Sprintf("0.0.0.0:%d", nodeDPPort))
	s = strings.ReplaceAll(s, "0.0.0.0:46358", fmt.Sprintf("0.0.0.0:%d", nodeCPPort))
	s = injectDataplaneServerHeaderMac(s, nodeDPPort, macKey)
	s = strings.ReplaceAll(s, "clients: []", fmt.Sprintf(
		"clients:\n        - endpoint: \"http://127.0.0.1:%d\"\n          tls:\n            insecure: true\n          header_mac_key: \"%s\"",
		peerDPPort, macKey,
	))
	dst := filepath.Join(dir, fileName)
	err = os.WriteFile(dst, []byte(s), 0o644)
	Expect(err).NotTo(HaveOccurred())
	return dst
}

var _ = Describe("Header Integrity 2-Node", func() {
	var (
		nodeASession   *gexec.Session
		nodeBSession   *gexec.Session
		clientASession *gexec.Session
		clientBSession *gexec.Session

		tempDir       string
		nodeAConfig   string
		nodeBConfig   string
		clientAConfig string
		clientBConfig string

		nodeAPort           int
		nodeBPort           int
		nodeAControllerPort int
		nodeBControllerPort int

		validKey = "01234567890123456789012345678901"
	)

	BeforeEach(func() {
		tempDir = newTempDir("slim-integration-header-integrity-2node-")
		nodeAPort = reservePort()
		nodeBPort = reservePort()
		nodeAControllerPort = reservePort()
		nodeBControllerPort = reservePort()
	})

	AfterEach(func() {
		terminateSession(clientASession, 2*time.Second)
		terminateSession(clientBSession, 2*time.Second)
		terminateSession(nodeASession, 10*time.Second)
		terminateSession(nodeBSession, 10*time.Second)

		if tempDir != "" {
			_ = os.RemoveAll(tempDir)
			tempDir = ""
		}
	})

	It("should deliver messages when header HMAC is valid across two nodes", func() {
		// Inter-node signing uses dataplane client config; inbound MAC uses dataplane server config.
		// Avoid replacing bare "insecure: true" — that also matches the client tls block and duplicates header_mac_key.
		nodeAConfig = writeHeaderIntegrityNodeConfig(tempDir, "node-a.yaml", nil, nodeAPort, nodeAControllerPort, nodeBPort, validKey)
		nodeBConfig = writeHeaderIntegrityNodeConfig(tempDir, "node-b.yaml", map[string]string{"slim/0": "slim/1"}, nodeBPort, nodeBControllerPort, nodeAPort, validKey)

		// Start nodes
		var err error
		nodeASession, err = gexec.Start(exec.Command(slimPath, "--config", nodeAConfig), GinkgoWriter, GinkgoWriter)
		Expect(err).NotTo(HaveOccurred())
		nodeBSession, err = gexec.Start(exec.Command(slimPath, "--config", nodeBConfig), GinkgoWriter, GinkgoWriter)
		Expect(err).NotTo(HaveOccurred())
		Eventually(nodeASession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))
		Eventually(nodeBSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

		// Setup clients
		clientAReplacements := map[string]string{
			"http://localhost:46357": fmt.Sprintf("http://localhost:%d", nodeAPort),
		}
		clientAConfig = writeTempConfig(tempDir, "./testdata/client.yaml", "client-a.yaml", clientAReplacements)
		clientBReplacements := map[string]string{
			"http://localhost:46357": fmt.Sprintf("http://localhost:%d", nodeBPort),
		}
		clientBConfig = writeTempConfig(tempDir, "./testdata/client.yaml", "client-b.yaml", clientBReplacements)

		// Start clients
		clientASession, err = gexec.Start(exec.Command(sdkMockPath, "--config", clientAConfig, "--local-name", "a", "--remote-name", "b", "--message", "hello-integrity"), GinkgoWriter, GinkgoWriter)
		Expect(err).NotTo(HaveOccurred())
		clientBSession, err = gexec.Start(exec.Command(sdkMockPath, "--config", clientBConfig, "--local-name", "b", "--remote-name", "a"), GinkgoWriter, GinkgoWriter)
		Expect(err).NotTo(HaveOccurred())

		// Restart Node A with SLIM_TEST_TAMPER_DESTINATION so the next outbound inter-node
		// publish gets a mismatched MAC (see slim_datapath send_msg_raw, debug builds only).
		terminateSession(nodeASession, 10*time.Second)
		nodeARestart := exec.Command(slimPath, "--config", nodeAConfig)
		nodeARestart.Env = envWithSlimHeaderTamper()
		nodeASession, err = gexec.Start(nodeARestart, GinkgoWriter, GinkgoWriter)
		Expect(err).NotTo(HaveOccurred())
		Eventually(nodeASession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

		// Send tampered message (stop the first client-a process before starting another)
		terminateSession(clientASession, 2*time.Second)
		clientASession, err = gexec.Start(exec.Command(sdkMockPath, "--config", clientAConfig, "--local-name", "a", "--remote-name", "b", "--message", "hello-tampered"), GinkgoWriter, GinkgoWriter)
		Expect(err).NotTo(HaveOccurred())

		// Verify Node B (second hop) detects integrity failure (tracing fmt goes to stdout in this suite).
		Eventually(nodeBSession.Out, 15*time.Second).Should(gbytes.Say("SLIM header integrity verification failed"), fmt.Sprintf("Verification check failed. stdout=%q stderr=%q", nodeBSession.Out.Contents(), nodeBSession.Err.Contents()))
		Consistently(clientBSession.Out, 5*time.Second).ShouldNot(gbytes.Say("hello-tampered"))
	})
})
