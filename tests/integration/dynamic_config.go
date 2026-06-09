// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
	"fmt"
	"net"
	"os"
	"path/filepath"
	"strings"
	"sync"

	ginkgo "github.com/onsi/ginkgo/v2"
	. "github.com/onsi/gomega"
)

var (
	portMutex sync.Mutex
	nextPort  int
)

func reservePort() int {
	portMutex.Lock()
	defer portMutex.Unlock()

	node := ginkgo.GinkgoParallelProcess()

	if nextPort == 0 {
		// Use a combination of Ginkgo parallel node and PID to minimize collisions
		// across parallel processes/runs.
		nextPort = 20000 + (node-1)*1000 + (os.Getpid()%100)*10
	}

	for {
		port := nextPort
		nextPort++

		// Ensure we don't go out of valid port range
		if nextPort > 65000 {
			nextPort = 20000 + (node-1)*1000 + (os.Getpid()%100)*10
		}

		// Double-check if the port is free to listen on
		addr := fmt.Sprintf("127.0.0.1:%d", port)
		l, err := net.Listen("tcp", addr)
		if err == nil {
			l.Close()
			return port
		}
	}
}

func newTempDir(prefix string) string {
	dir, err := os.MkdirTemp("", prefix)
	Expect(err).NotTo(HaveOccurred())
	return dir
}

func writeTempConfig(dir, srcPath, dstName string, replacements map[string]string) string {
	contents, err := os.ReadFile(srcPath)
	Expect(err).NotTo(HaveOccurred())

	updated := string(contents)
	for oldValue, newValue := range replacements {
		updated = strings.ReplaceAll(updated, oldValue, newValue)
	}

	dstPath := filepath.Join(dir, dstName)
	err = os.WriteFile(dstPath, []byte(updated), 0o644)
	Expect(err).NotTo(HaveOccurred())

	return dstPath
}

func writeTempConfigNearSource(srcPath, pattern string, replacements map[string]string) string {
	contents, err := os.ReadFile(srcPath)
	Expect(err).NotTo(HaveOccurred())

	updated := string(contents)
	for oldValue, newValue := range replacements {
		updated = strings.ReplaceAll(updated, oldValue, newValue)
	}

	file, err := os.CreateTemp(filepath.Dir(srcPath), pattern)
	Expect(err).NotTo(HaveOccurred())
	defer file.Close()

	_, err = file.WriteString(updated)
	Expect(err).NotTo(HaveOccurred())

	return file.Name()
}
