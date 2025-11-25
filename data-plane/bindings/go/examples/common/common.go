// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Package common provides shared helper utilities for SLIM Go binding examples.
//
// This package provides:
//   - Identity string parsing (org/namespace/app)
package common

import (
	"fmt"
	"strings"

	slim "github.com/agntcy/slim/bindings/generated/slim_service"
)

// SplitID splits an ID of form organization/namespace/application (or channel).
//
// Args:
//
//	id: String in the canonical 'org/namespace/app-or-stream' format.
//
// Returns:
//
//	Name: Constructed identity object.
//	error: If the id cannot be split into exactly three segments.
func SplitID(id string) (slim.Name, error) {
	parts := strings.Split(id, "/")
	if len(parts) != 3 {
		return slim.Name{}, fmt.Errorf("IDs must be in the format organization/namespace/app-or-stream, got: %s", id)
	}
	return slim.Name{
		Components: []string{parts[0], parts[1], parts[2]},
		Id:         nil,
	}, nil
}
