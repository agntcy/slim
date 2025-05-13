// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
	"os/exec"
	"path/filepath"
	"strings"
	"time"

	. "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
	"github.com/onsi/gomega/gbytes"
	"github.com/onsi/gomega/gexec"
)

var (
	target string

	gatewayPath string
	sdkMockPath string
	agpctlPath  string

	serverASession *gexec.Session
	serverBSession *gexec.Session
)

var _ = BeforeSuite(func() {
	// determine build target
	out, err := exec.Command("rustc", "-vV").Output()
	Expect(err).NotTo(HaveOccurred())

	for _, line := range strings.Split(string(out), "\n") {
		if strings.HasPrefix(line, "host: ") {
			target = strings.TrimSpace(strings.TrimPrefix(line, "host: "))
			break
		}
	}
	Expect(target).NotTo(BeEmpty(), "failed to parse rustc host target")

	// set binary paths
	gatewayPath = filepath.Join("..", "..", "data-plane", "target", target, "debug", "gateway")
	sdkMockPath = filepath.Join("..", "..", "data-plane", "target", target, "debug", "sdk-mock")
	agpctlPath = filepath.Join("..", "..", ".dist", "bin", "agpctl")

	// start gateways
	var errA, errB error
	serverASession, errA = gexec.Start(
		exec.Command(gatewayPath, "--config", "./testdata/server-a-config.yaml"),
		GinkgoWriter, GinkgoWriter,
	)
	serverBSession, errB = gexec.Start(
		exec.Command(gatewayPath, "--config", "./testdata/server-b-config.yaml"),
		GinkgoWriter, GinkgoWriter,
	)
	Expect(errA).NotTo(HaveOccurred())
	Expect(errB).NotTo(HaveOccurred())

	// wait for gateways to start
	time.Sleep(2000 * time.Millisecond)

	// add routes
	Expect(exec.Command(agpctlPath,
		"route", "add", "org/default/b/0",
		"via", "localhost:46367",
		"-s", "127.0.0.1:46358",
	).Run()).To(Succeed())

	Expect(exec.Command(agpctlPath,
		"route", "add", "org/default/a/0",
		"via", "localhost:46357",
		"-s", "127.0.0.1:46368",
	).Run()).To(Succeed())
})

var _ = AfterSuite(func() {
	// terminate gateways
	serverASession.Terminate().Wait(3 * time.Second)
	serverBSession.Terminate().Wait(3 * time.Second)
})

var _ = Describe("Routing", func() {
	var (
		clientASession *gexec.Session
		clientBSession *gexec.Session
	)

	AfterEach(func() {
		// terminate agents
		if clientASession != nil {
			clientASession.Terminate().Wait(2 * time.Second)
		}
		if clientBSession != nil {
			clientBSession.Terminate().Wait(2 * time.Second)
		}
	})

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
})
