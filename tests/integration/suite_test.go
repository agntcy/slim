// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
	"os/exec"
	"path/filepath"
	"strings"
	"testing"
	"time"

	. "github.com/onsi/ginkgo/v2"
	ginkgo "github.com/onsi/ginkgo/v2"
	"github.com/onsi/gomega"
	. "github.com/onsi/gomega"
	"github.com/onsi/gomega/gexec"
)

var (
	target string

	slimPath         string
	sdkMockPath      string
	slimctlPath      string
	controlPlanePath string

	serverASession      *gexec.Session
	serverBSession      *gexec.Session
	controlPlaneSession *gexec.Session
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
	slimPath = filepath.Join("..", "..", "data-plane", "target", target, "debug", "slim")
	sdkMockPath = filepath.Join("..", "..", "data-plane", "target", target, "debug", "sdk-mock")
	slimctlPath = filepath.Join("..", "..", ".dist", "bin", "slimctl")
	controlPlanePath = filepath.Join("..", "..", ".dist", "bin", "control-plane")
})

var _ = AfterSuite(func() {
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
})

func TestIntegration(t *testing.T) {
	gomega.RegisterFailHandler(ginkgo.Fail)
	ginkgo.RunSpecs(t, "Run integration tests")
}
