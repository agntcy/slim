// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
	"os/exec"
	"time"

	. "github.com/onsi/gomega"
)

func runCombinedOutputWithRetry(timeout time.Duration, cmd func() *exec.Cmd) []byte {
	deadline := time.Now().Add(timeout)
	var lastOut []byte
	var lastErr error
	var lastCmd string

	for {
		command := cmd()
		lastCmd = command.String()
		lastOut, lastErr = command.CombinedOutput()
		if lastErr == nil {
			return lastOut
		}
		if time.Now().After(deadline) {
			break
		}
		time.Sleep(200 * time.Millisecond)
	}

	Expect(lastErr).NotTo(HaveOccurred(), "command failed after retry: %s\nerror: %v\noutput:\n%s", lastCmd, lastErr, string(lastOut))
	return lastOut
}
