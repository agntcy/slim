// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
	"fmt"
	"os"
	"os/exec"
	"path/filepath"
	"strings"
	"testing"

	. "github.com/onsi/ginkgo/v2"
	ginkgo "github.com/onsi/ginkgo/v2"
	"github.com/onsi/gomega"
	. "github.com/onsi/gomega"
)

var (
	target string

	suiteName = "Run integration tests"

	slimPath         string
	sdkMockPath      string
	clientPath       string
	slimctlPath      string
	controlPlanePath string
)

func mustAbs(p string) string {
	abs, err := filepath.Abs(p)
	Expect(err).NotTo(HaveOccurred(), fmt.Sprintf("failed to resolve absolute path for %s", p))
	return abs
}

func setBinaryPaths(target string) {
	dataPlaneTarget := filepath.Join("..", "..", "data-plane", "target", target, "debug")
	slimPath = mustAbs(filepath.Join(dataPlaneTarget, "slim"))
	sdkMockPath = mustAbs(filepath.Join(dataPlaneTarget, "sdk-mock"))
	clientPath = mustAbs(filepath.Join(dataPlaneTarget, "client"))
	distBin := filepath.Join("..", "..", ".dist", "bin")

	if override := strings.TrimSpace(os.Getenv("SLIMCTL_PATH")); override != "" {
		slimctlPath = mustAbs(override)
	} else {
		slimctlPath = mustAbs(filepath.Join(distBin, "slimctl"))
	}
	controlPlanePath = mustAbs(filepath.Join(distBin, "control-plane"))
}

var _ = BeforeSuite(func() {
	fmt.Fprintf(GinkgoWriter, "[integration] BeforeSuite (%s): start\n", suiteName)
	// determine build target
	out, err := exec.Command("rustc", "-vV").Output()
	Expect(err).NotTo(HaveOccurred())

	for _, line := range strings.Split(string(out), "\n") {
		if after, ok := strings.CutPrefix(line, "host: "); ok {
			target = strings.TrimSpace(after)
			break
		}
	}
	Expect(target).NotTo(BeEmpty(), "failed to parse rustc host target")

	// set binary paths
	setBinaryPaths(target)
})

var _ = AfterSuite(func() {
})

func TestIntegration(t *testing.T) {
	gomega.RegisterFailHandler(ginkgo.Fail)
	ginkgo.RunSpecs(t, suiteName)
}
