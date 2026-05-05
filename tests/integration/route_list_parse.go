// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package integration

import (
	"fmt"
	"strings"
)

// extractSubscriptionName finds a subscription matching prefix in slimctl
// `controller route list` output and returns "prefix/id" for channel APIs.
// The id= segment uses Rust Debug formatting and may contain spaces
// (e.g. Some(UInt64Value { value: 123 })), so we slice between " id=" and " local=".
func extractSubscriptionName(routeListOutput, prefix string) string {
	for _, line := range strings.Split(routeListOutput, "\n") {
		if !strings.Contains(line, prefix) {
			continue
		}
		const idMarker = " id="
		const localMarker = " local="
		idAt := strings.Index(line, idMarker)
		if idAt < 0 {
			continue
		}
		nameWithoutID := strings.TrimSpace(line[:idAt])
		if nameWithoutID != prefix {
			continue
		}
		localAt := strings.Index(line, localMarker)
		if localAt <= idAt {
			continue
		}
		idRaw := strings.TrimSpace(line[idAt+len(idMarker):localAt])
		idStr := normalizeRustDebugUInt64(idRaw)
		if idStr == "" {
			continue
		}
		return fmt.Sprintf("%s/%s", nameWithoutID, idStr)
	}
	return ""
}

func normalizeRustDebugUInt64(s string) string {
	s = strings.TrimSpace(s)
	if s == "" || s == "None" || s == "null" {
		return "0"
	}
	if after, ok := strings.CutPrefix(s, "Some("); ok {
		s = strings.TrimSuffix(strings.TrimSpace(after), ")")
		s = strings.TrimSpace(s)
	}
	if idx := strings.Index(s, "value:"); idx >= 0 {
		rest := strings.TrimSpace(s[idx+len("value:"):])
		end := len(rest)
		for i, r := range rest {
			if r < '0' || r > '9' {
				end = i
				break
			}
		}
		if end > 0 {
			return rest[:end]
		}
	}
	s = strings.Trim(s, "() ")
	if s == "" {
		return "0"
	}
	return s
}
