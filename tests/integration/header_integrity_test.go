// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

// Header MAC (HMAC-SHA256) checks for inter-node traffic.
//
// This file combines:
//   1) A Go integration topology: two slim dataplane processes federate and complete link
//      negotiation (ECDH-derived per-link MAC keys on the gRPC stream), matching production wiring.
//   2) Targeted Rust unit tests in agntcy-slim-datapath that exercise MessageProcessor's
//      verify_remote_header_mac path: a correctly signed publish verifies, and mutating the
//      destination string after signing fails verification (same tamper shape as
//      SLIM_TEST_TAMPER_DESTINATION on outbound remote sends).
//
// End-to-end sdk-mock publish across two nodes is not used here: Point-to-point session discovery
// from a publisher on node A still requires subscription state for the peer on A's dataplane, and
// forwarded peer self-subscriptions are not registered under the peer's full name on the hub today,
// so discovery does not resolve across this hop without further dataplane work.

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"time"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	"github.com/onsi/gomega/gbytes"
	"github.com/onsi/gomega/gexec"
)

func twoNodeFederationReplacements(nodeAPort, nodeBPort int) map[string]string {
	return map[string]string{
		"0.0.0.0:46357":          fmt.Sprintf("0.0.0.0:%d", nodeAPort),
		"0.0.0.0:46481":          fmt.Sprintf("0.0.0.0:%d", nodeBPort),
		"http://localhost:46480": fmt.Sprintf("http://localhost:%d", nodeAPort),
		"http://localhost:46357": fmt.Sprintf("http://localhost:%d", nodeAPort),
		"http://localhost:46481": fmt.Sprintf("http://localhost:%d", nodeBPort),
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

	runDataplaneHeaderMacTests := func() {
		dataPlaneDir, err := filepath.Abs(filepath.Join("..", "..", "data-plane"))
		Expect(err).NotTo(HaveOccurred())
		cmd := exec.Command(
			"cargo", "test", "-p", "agntcy-slim-datapath", "verify_remote_header_mac", "--",
		)
		cmd.Dir = dataPlaneDir
		out, err := cmd.CombinedOutput()
		Expect(err).NotTo(HaveOccurred(), string(out))
	}

	It("completes link negotiation on a federated pair and dataplane tests confirm header MAC behavior", func() {
		nodeAPort := reservePort()
		nodeBPort := reservePort()
		repl := twoNodeFederationReplacements(nodeAPort, nodeBPort)

		nodeAConfig := writeTempConfig(tempDir, "./testdata/server.yaml", "node-a.yaml", repl)
		nodeBConfig := writeTempConfig(tempDir, "./testdata/link-neg-node-with-client-config.yaml", "node-b.yaml", repl)

		nodeASession, err := gexec.Start(
			exec.Command(slimPath, "--config", nodeAConfig),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(err).NotTo(HaveOccurred())
		defer terminateSession(nodeASession, 5*time.Second)
		Eventually(nodeASession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))

		nodeBSession, err := gexec.Start(
			exec.Command(slimPath, "--config", nodeBConfig),
			GinkgoWriter, GinkgoWriter,
		)
		Expect(err).NotTo(HaveOccurred())
		defer terminateSession(nodeBSession, 5*time.Second)
		Eventually(nodeBSession.Out, 15*time.Second).Should(gbytes.Say("dataplane server started"))
		Eventually(nodeASession.Out, 10*time.Second).Should(gbytes.Say("received link negotiation"))
		Eventually(nodeBSession.Out, 10*time.Second).Should(gbytes.Say("received link negotiation"))

		runDataplaneHeaderMacTests()
	})
})
