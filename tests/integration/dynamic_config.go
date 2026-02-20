// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
	"net"
	"os"
	"path/filepath"
	"strings"

	. "github.com/onsi/gomega"
)

func reservePort() int {
	listener, err := net.Listen("tcp", "127.0.0.1:0")
	Expect(err).NotTo(HaveOccurred())
	defer listener.Close()

	addr, ok := listener.Addr().(*net.TCPAddr)
	Expect(ok).To(BeTrue())
	return addr.Port
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
