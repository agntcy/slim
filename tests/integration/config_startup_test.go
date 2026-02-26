// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

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

// (removed helper functions extractEndpoints and dialEndpoints; direct connection verification no longer required)

// resolveConfigPaths determines which config files to test.
// Priority:
// 1. Environment variable SLIM_CONFIGS (comma-separated paths)
// 2. Default curated list of server-centric configuration files.
//
// Paths can be absolute or relative; relative paths are resolved
// against the project root (assuming test is run from integration dir).
type configCase struct {
	ServerPath string
	ClientPath string
}

func resolveConfigCases() []configCase {
	// helper to convert path to absolute (fallback to original on error)
	abs := func(p string) string {
		if ap, err := filepath.Abs(p); err == nil {
			return ap
		}
		return p
	}

	// Build list of candidate directories
	var dirs []string
	configRoot := filepath.Join("..", "..", "data-plane", "config")
	entries, err := os.ReadDir(configRoot)
	if err != nil {
		return nil
	}
	for _, e := range entries {
		if e.IsDir() {
			dirs = append(dirs, filepath.Join(configRoot, e.Name()))
		}
	}

	// De-duplicate and validate each directory; include only those with both server-config.yaml and client-config.yaml.
	seen := make(map[string]struct{})
	var out []configCase
	for _, d := range dirs {
		if _, ok := seen[d]; ok {
			continue
		}
		seen[d] = struct{}{}

		info, err := os.Stat(d)
		if err != nil || !info.IsDir() {
			fmt.Println(GinkgoWriter, "Warning: config directory invalid:", d)
			continue
		}

		serverCfg := filepath.Join(d, "server-config.yaml")
		clientCfg := filepath.Join(d, "client-config.yaml")
		if _, errS := os.Stat(serverCfg); errS != nil {
			fmt.Println(GinkgoWriter, "Warning: server config missing:", serverCfg)
			continue
		}
		if _, errC := os.Stat(clientCfg); errC != nil {
			fmt.Println(GinkgoWriter, "Warning: client config missing:", clientCfg)
			continue
		}

		out = append(out, configCase{
			ServerPath: abs(serverCfg),
			ClientPath: abs(clientCfg),
		})
	}

	return out
}

var _ = Describe("SLIM server + client connection using configuration files", func() {

	cases := resolveConfigCases()

	// declare some environment variables needed by some configs
	os.Setenv("PASSWORD", "password")

	for _, c := range cases {
		It(fmt.Sprintf("server and client start and connect (config dir: %s)", filepath.Dir(c.ServerPath)), func() {
			fmt.Println(GinkgoWriter, "Testing config - server:", c.ServerPath, "client:", c.ClientPath)

			dataPlanePort := reservePort()
			replacements := map[string]string{
				"0.0.0.0:46357":           fmt.Sprintf("0.0.0.0:%d", dataPlanePort),
				"http://localhost:46357":  fmt.Sprintf("http://localhost:%d", dataPlanePort),
				"http://127.0.0.1:46357":  fmt.Sprintf("http://127.0.0.1:%d", dataPlanePort),
				"https://localhost:46357": fmt.Sprintf("https://localhost:%d", dataPlanePort),
				"https://127.0.0.1:46357": fmt.Sprintf("https://127.0.0.1:%d", dataPlanePort),
			}

			serverConfig := writeTempConfigNearSource(c.ServerPath, "tmp-server-config-*.yaml", replacements)
			clientConfig := writeTempConfigNearSource(c.ClientPath, "tmp-client-config-*.yaml", replacements)
			defer func() {
				_ = os.Remove(serverConfig)
				_ = os.Remove(clientConfig)
			}()

			// Server config must exist
			serverInfo, err := os.Stat(c.ServerPath)
			Expect(err).NotTo(HaveOccurred(), fmt.Sprintf("server configuration file not found: %s", c.ServerPath))
			Expect(serverInfo.IsDir()).To(BeFalse(), fmt.Sprintf("server config path must be a file: %s", c.ServerPath))

			// Client config must exist
			clientInfo, errClient := os.Stat(c.ClientPath)
			if errClient != nil || clientInfo.IsDir() {
				Skip(fmt.Sprintf("client config missing for server config %s (expected %s)", c.ServerPath, c.ClientPath))
			}

			dataPlaneDir := filepath.Join("..", "..", "data-plane")

			// Start server SLIM node
			serverCmd := exec.Command(slimPath, "--config", serverConfig)
			serverCmd.Dir = dataPlaneDir
			serverSession, err := gexec.Start(serverCmd, GinkgoWriter, GinkgoWriter)
			Expect(err).NotTo(HaveOccurred(), fmt.Sprintf("failed to start server SLIM with config %s", c.ServerPath))
			Eventually(serverSession, 15*time.Second).Should(gbytes.Say("dataplane server started"))

			// Ensure server cleanup
			defer func() {
				terminateSession(serverSession, 5*time.Second)
			}()

			// Start client SLIM node
			clientCmd := exec.Command(slimPath, "--config", clientConfig)
			clientCmd.Dir = dataPlaneDir
			clientSession, errClientStart := gexec.Start(clientCmd, GinkgoWriter, GinkgoWriter)
			Expect(errClientStart).NotTo(HaveOccurred(), fmt.Sprintf("failed to start client SLIM with config %s", c.ClientPath))

			// Ensure client cleanup
			defer func() {
				terminateSession(clientSession, 5*time.Second)
			}()

			Eventually(clientSession, 15*time.Second).Should(gbytes.Say("client connected"))
		})
	}
})
