// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
	"fmt"
	"io"
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

// Paths resolved in initSlimrpcPaths relative to the integration test directory.
var (
	// Root of the repository (two levels up from tests/integration/).
	repoRoot string

	// Language-specific working directories (resolved from repoRoot).
	goSlimrpcDir      string
	pythonBindingsDir string
	javaSlimrpcDir    string
	dotnetDir         string

	// Pre-built Go binaries (built by the Taskfile before the test runs).
	goRPCServerPath      string
	goRPCClientPath      string
	goRPCGroupClientPath string
)

// All languages under test.
var langs = []string{"go", "python", "java", "csharp"}

func initSlimrpcPaths() {
	repoRoot = mustAbs(filepath.Join("..", ".."))
	goSlimrpcDir = filepath.Join(repoRoot, "data-plane", "bindings", "go", "examples", "slimrpc", "simple")
	pythonBindingsDir = filepath.Join(repoRoot, "data-plane", "bindings", "python")
	javaSlimrpcDir = filepath.Join(repoRoot, "data-plane", "bindings", "java", "examples", "slimrpc", "simple")
	dotnetDir = filepath.Join(repoRoot, "data-plane", "bindings", "dotnet")

	goRPCServerPath = filepath.Join(goSlimrpcDir, "slimrpc-go-server")
	goRPCClientPath = filepath.Join(goSlimrpcDir, "slimrpc-go-client")
	goRPCGroupClientPath = filepath.Join(goSlimrpcDir, "slimrpc-go-group-client")
}

// ---------------------------------------------------------------------------
// Language-specific command builders
// ---------------------------------------------------------------------------

// rpcServerCmd returns an exec.Cmd that starts an RPC server for the given
// language. instance is the SLIM app name (e.g. "server" or "server1").
func rpcServerCmd(lang, slimAddr, instance string) *exec.Cmd {
	switch lang {
	case "go":
		return exec.Command(goRPCServerPath, "--server", slimAddr, "-instance="+instance)
	case "python":
		cmd := exec.Command("uv", "run", "slim-bindings-simple-rpc-server",
			"--server", slimAddr, "--instance", instance)
		cmd.Dir = pythonBindingsDir
		return cmd
	case "java":
		cmd := exec.Command("mvn", "exec:java@server", "-q",
			fmt.Sprintf("-Dexec.args=--server %s --instance %s", slimAddr, instance))
		cmd.Dir = javaSlimrpcDir
		return cmd
	case "csharp":
		cmd := exec.Command("dotnet", "run",
			"--project", "Slim.Examples.SlimRpc",
			"--no-build", "--configuration", "Release",
			"--", "--mode", "server",
			"--server", slimAddr,
			"--instance", instance)
		cmd.Dir = dotnetDir
		return cmd
	default:
		Fail(fmt.Sprintf("unknown server language: %s", lang))
		return nil
	}
}

// rpcClientCmd returns an exec.Cmd that runs a unicast RPC client.
func rpcClientCmd(lang, slimAddr string) *exec.Cmd {
	switch lang {
	case "go":
		return exec.Command(goRPCClientPath, "--server", slimAddr)
	case "python":
		cmd := exec.Command("uv", "run", "slim-bindings-simple-rpc-client",
			"--server", slimAddr)
		cmd.Dir = pythonBindingsDir
		return cmd
	case "java":
		cmd := exec.Command("mvn", "exec:java@client", "-q",
			fmt.Sprintf("-Dexec.args=--server %s", slimAddr))
		cmd.Dir = javaSlimrpcDir
		return cmd
	case "csharp":
		cmd := exec.Command("dotnet", "run",
			"--project", "Slim.Examples.SlimRpc",
			"--no-build", "--configuration", "Release",
			"--", "--mode", "client",
			"--server", slimAddr)
		cmd.Dir = dotnetDir
		return cmd
	default:
		Fail(fmt.Sprintf("unknown client language: %s", lang))
		return nil
	}
}

// rpcGroupClientCmd returns an exec.Cmd that runs a multicast (group) RPC
// client targeting the given comma-separated server instance names.
func rpcGroupClientCmd(lang, slimAddr, servers string) *exec.Cmd {
	switch lang {
	case "go":
		return exec.Command(goRPCGroupClientPath,
			"--server", slimAddr, "--servers", servers)
	case "python":
		cmd := exec.Command("uv", "run", "slim-bindings-simple-rpc-client-group",
			"--server", slimAddr, "--servers", servers)
		cmd.Dir = pythonBindingsDir
		return cmd
	case "java":
		cmd := exec.Command("mvn", "exec:java@group-client", "-q",
			fmt.Sprintf("-Dexec.args=--server %s --servers %s", slimAddr, servers))
		cmd.Dir = javaSlimrpcDir
		return cmd
	case "csharp":
		cmd := exec.Command("dotnet", "run",
			"--project", "Slim.Examples.SlimRpc",
			"--no-build", "--configuration", "Release",
			"--", "--mode", "group-client",
			"--server", slimAddr,
			"--servers", servers)
		cmd.Dir = dotnetDir
		return cmd
	default:
		Fail(fmt.Sprintf("unknown group-client language: %s", lang))
		return nil
	}
}

// toolAvailable returns true when the named binary is on PATH.
func toolAvailable(name string) bool {
	_, err := exec.LookPath(name)
	return err == nil
}

// requiredTool returns the tool binary name needed for a given language.
// Go uses pre-built binaries so no runtime tool is needed.
func requiredTool(lang string) string {
	switch lang {
	case "python":
		return "uv"
	case "java":
		return "mvn"
	case "csharp":
		return "dotnet"
	default:
		return ""
	}
}

// skipIfToolMissing skips the current spec if any of the languages in the
// list requires a tool that is not on PATH.
func skipIfToolMissing(languages ...string) {
	for _, lang := range languages {
		if tool := requiredTool(lang); tool != "" && !toolAvailable(tool) {
			Skip(fmt.Sprintf("skipping: %q not found on PATH (needed for %s)", tool, lang))
		}
	}
}

// startRPCServer starts the RPC server for the given language and waits
// until it prints the SLIM_RPC_SERVER_READY marker.
// Output goes to GinkgoWriter (hidden on success, shown on failure).
// Returns the session; the caller is responsible for terminating it.
func startRPCServer(lang, slimAddr, instance string) *gexec.Session {
	fmt.Fprintf(GinkgoWriter, "[test] Starting %s RPC server (instance=%s)...\n", lang, instance)

	// Tee into a gbytes.Buffer for matching AND GinkgoWriter for
	// automatic display on failure.
	serverOut := gbytes.NewBuffer()
	tee := io.MultiWriter(serverOut, GinkgoWriter)
	session, err := gexec.Start(
		rpcServerCmd(lang, slimAddr, instance),
		tee, tee,
	)
	Expect(err).NotTo(HaveOccurred(),
		fmt.Sprintf("failed to start %s RPC server (instance=%s)", lang, instance))

	// Every language server prints this exact marker once it is serving.
	Eventually(serverOut, 30*time.Second).Should(
		gbytes.Say("SLIM_RPC_SERVER_READY"),
		fmt.Sprintf("%s RPC server (instance=%s) did not become ready in time", lang, instance),
	)

	// Give the server a moment to complete its registration with SLIM.
	time.Sleep(5 * time.Second)

	Expect(session.ExitCode()).To(Equal(-1),
		fmt.Sprintf("%s RPC server (instance=%s) exited prematurely", lang, instance))

	fmt.Fprintf(GinkgoWriter, "[test] %s RPC server (instance=%s) ready\n", lang, instance)
	return session
}

// ---------------------------------------------------------------------------
// Test suite
// ---------------------------------------------------------------------------

var _ = Describe("SlimRPC cross-language", Ordered, func() {
	var (
		slimSession *gexec.Session
		slimAddr    string
		tempDir     string
	)

	BeforeAll(func() {
		initSlimrpcPaths()

		// Reserve a random port so we never conflict with other tests or
		// a developer's local SLIM instance.
		port := reservePort()
		slimAddr = fmt.Sprintf("http://localhost:%d", port)

		fmt.Fprintf(GinkgoWriter, "[test] Starting SLIM data-plane on port %d\n", port)

		// Write a temporary SLIM config that listens on the chosen port.
		replacements := map[string]string{
			"0.0.0.0:46357": fmt.Sprintf("0.0.0.0:%d", port),
		}
		tempDir = newTempDir("slim-slimrpc-cross-lang-")
		slimConfig := writeTempConfig(
			tempDir,
			"./testdata/server.yaml",
			"slimrpc-server-config.yaml",
			replacements,
		)

		// Start the single SLIM data-plane node.
		cmd := exec.Command(slimPath, "--config", slimConfig)
		var err error
		slimSession, err = gexec.Start(cmd, GinkgoWriter, GinkgoWriter)
		Expect(err).NotTo(HaveOccurred(), "failed to start SLIM data-plane")

		Eventually(slimSession.Out, 15*time.Second).Should(
			gbytes.Say("dataplane server started"),
			"SLIM data-plane did not become ready in time",
		)

		fmt.Fprintf(GinkgoWriter, "[test] SLIM data-plane ready on port %d\n", port)
	})

	AfterAll(func() {
		fmt.Fprintf(GinkgoWriter, "[test] Shutting down SLIM data-plane\n")
		terminateSession(slimSession, 10*time.Second)
		if tempDir != "" {
			_ = os.RemoveAll(tempDir)
		}
	})

	// =================================================================
	// Unicast: every client language calls every server language (4x4).
	// =================================================================
	Context("unicast", func() {
		for _, sLang := range langs {
			sLang := sLang // capture

			Context(sLang+" server", Ordered, func() {
				var serverSession *gexec.Session

				BeforeAll(func() {
					skipIfToolMissing(sLang)
					serverSession = startRPCServer(sLang, slimAddr, "server")
				})

				AfterAll(func() {
					fmt.Fprintf(GinkgoWriter, "[test] Stopping %s server\n", sLang)
					terminateSession(serverSession, 5*time.Second)
				})

				for _, cLang := range langs {
					cLang := cLang // capture

					It(fmt.Sprintf("<- %s client", cLang), func() {
						skipIfToolMissing(cLang)

						fmt.Fprintf(GinkgoWriter, "[test] Running %s client -> %s server\n", cLang, sLang)

						clientOut := gbytes.NewBuffer()
						clientTee := io.MultiWriter(clientOut, GinkgoWriter)
						clientSession, err := gexec.Start(
							rpcClientCmd(cLang, slimAddr),
							clientTee, clientTee,
						)
						Expect(err).NotTo(HaveOccurred(),
							fmt.Sprintf("failed to start %s RPC client", cLang))

						// Verify the client printed the STARTED marker.
						Eventually(clientOut, 30*time.Second).Should(
							gbytes.Say("SLIM_RPC_CLIENT_STARTED"),
							fmt.Sprintf("%s client did not print STARTED marker", cLang))

						fmt.Fprintf(GinkgoWriter, "[test] %s client started, waiting for completion\n", cLang)

						// Wait for the client to finish all RPC patterns and exit 0.
						Eventually(clientSession, 60*time.Second).Should(gexec.Exit(0),
							fmt.Sprintf("%s client -> %s server did not exit 0", cLang, sLang))

						// Verify the DONE marker and all four RPC patterns.
						output := string(clientOut.Contents())
						Expect(output).To(ContainSubstring("SLIM_RPC_CLIENT_DONE"),
							fmt.Sprintf("%s client did not print DONE marker", cLang))
						Expect(output).To(ContainSubstring("Unary-Unary"),
							fmt.Sprintf("%s client output missing Unary-Unary section", cLang))
						Expect(output).To(ContainSubstring("Unary-Stream"),
							fmt.Sprintf("%s client output missing Unary-Stream section", cLang))
						Expect(output).To(ContainSubstring("Stream-Unary"),
							fmt.Sprintf("%s client output missing Stream-Unary section", cLang))
						Expect(output).To(ContainSubstring("Stream-Stream"),
							fmt.Sprintf("%s client output missing Stream-Stream section", cLang))

						fmt.Fprintf(GinkgoWriter, "[test] %s client -> %s server: OK\n", cLang, sLang)
					})
				}
			})
		}
	})

	// =================================================================
	// Multicast: all four servers run simultaneously, each language's
	// group client opens a multicast session to all four.
	// =================================================================
	Context("multicast", Ordered, func() {
		// Each language gets a unique server instance name.
		instanceOf := map[string]string{
			"go":     "server1",
			"python": "server2",
			"java":   "server3",
			"csharp": "server4",
		}

		// Comma-separated list passed to --servers.
		allInstances := strings.Join([]string{
			instanceOf["go"],
			instanceOf["python"],
			instanceOf["java"],
			instanceOf["csharp"],
		}, ",")

		var serverSessions []*gexec.Session

		BeforeAll(func() {
			skipIfToolMissing(langs...)

			fmt.Fprintf(GinkgoWriter, "[test] Starting all 4 RPC servers for multicast test\n")
			for _, lang := range langs {
				session := startRPCServer(lang, slimAddr, instanceOf[lang])
				serverSessions = append(serverSessions, session)
			}
			fmt.Fprintf(GinkgoWriter, "[test] All 4 servers ready: %s\n", allInstances)
		})

		AfterAll(func() {
			fmt.Fprintf(GinkgoWriter, "[test] Stopping all multicast servers\n")
			for _, s := range serverSessions {
				terminateSession(s, 5*time.Second)
			}
		})

		for _, cLang := range langs {
			cLang := cLang // capture

			It(fmt.Sprintf("%s group client -> all servers", cLang), func() {
				skipIfToolMissing(cLang)

				fmt.Fprintf(GinkgoWriter, "[test] Running %s group client -> [%s]\n", cLang, allInstances)

				clientOut := gbytes.NewBuffer()
				clientTee := io.MultiWriter(clientOut, GinkgoWriter)
				clientSession, err := gexec.Start(
					rpcGroupClientCmd(cLang, slimAddr, allInstances),
					clientTee, clientTee,
				)
				Expect(err).NotTo(HaveOccurred(),
					fmt.Sprintf("failed to start %s group client", cLang))

				// Verify the group client printed the STARTED marker.
				Eventually(clientOut, 30*time.Second).Should(
					gbytes.Say("SLIM_RPC_GROUP_CLIENT_STARTED"),
					fmt.Sprintf("%s group client did not print STARTED marker", cLang))

				fmt.Fprintf(GinkgoWriter, "[test] %s group client started, waiting for completion\n", cLang)

				// Wait for the group client to finish all multicast patterns and exit 0.
				Eventually(clientSession, 60*time.Second).Should(gexec.Exit(0),
					fmt.Sprintf("%s group client did not exit 0", cLang))

				// Verify the DONE marker and all four multicast RPC patterns.
				output := string(clientOut.Contents())
				Expect(output).To(ContainSubstring("SLIM_RPC_GROUP_CLIENT_DONE"),
					fmt.Sprintf("%s group client did not print DONE marker", cLang))
				Expect(output).To(ContainSubstring("Multicast Unary-Unary"),
					fmt.Sprintf("%s group client output missing Multicast Unary-Unary section", cLang))
				Expect(output).To(ContainSubstring("Multicast Unary-Stream"),
					fmt.Sprintf("%s group client output missing Multicast Unary-Stream section", cLang))
				Expect(output).To(ContainSubstring("Multicast Stream-Unary"),
					fmt.Sprintf("%s group client output missing Multicast Stream-Unary section", cLang))
				Expect(output).To(ContainSubstring("Multicast Stream-Stream"),
					fmt.Sprintf("%s group client output missing Multicast Stream-Stream section", cLang))

				// Verify that EVERY server actually responded.
				for _, inst := range []string{
					instanceOf["go"],
					instanceOf["python"],
					instanceOf["java"],
					instanceOf["csharp"],
				} {
					Expect(output).To(ContainSubstring("agntcy/grpc/"+inst),
						fmt.Sprintf("%s group client did not receive any response from %s", cLang, inst))
				}

				fmt.Fprintf(GinkgoWriter, "[test] %s group client -> all servers: OK\n", cLang)
			})
		}
	})
})
