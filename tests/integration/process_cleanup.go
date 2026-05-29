// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
	"time"

	"github.com/onsi/gomega/gexec"
)

func terminateSession(session *gexec.Session, timeout time.Duration) {
	if session == nil {
		return
	}

	session.Terminate()
	if !waitForExit(session, timeout) {
		// Force cleanup to avoid port conflicts between specs.
		session.Kill()
		_ = waitForExit(session, 5*time.Second)
	}
}

func waitForExit(session *gexec.Session, timeout time.Duration) bool {
	select {
	case <-session.Exited:
		return true
	case <-time.After(timeout):
		return false
	}
}
