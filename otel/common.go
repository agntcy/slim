// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Package common provides shared helper utilities for SLIM Go binding examples.
//
// This package provides:
//   - Identity string parsing (org/namespace/app)
//   - App creation and connection helper
//   - Default configuration values
package otel

import (
	"fmt"
	"strings"

	slim "github.com/agntcy/slim-bindings-go"
)

type SignalType string

// SignalType represents the type of signal to be exported
const (
	SignalTraces  SignalType = "traces"
	SignalMetrics SignalType = "metrics"
	SignalLogs    SignalType = "logs"
)

// ExtractSignalType extracts the signal type from a string
// by checking its suffix.
//
// Args:
//
//	component: The component string to analyze.
//
// Returns:
//
//	SignalType: The detected signal type (traces, metrics, or logs)
//	error: If no known suffix is found
func ExtractSignalType(component string) (SignalType, error) {
	switch {
	case strings.HasSuffix(component, string(SignalTraces)):
		return SignalTraces, nil
	case strings.HasSuffix(component, string(SignalMetrics)):
		return SignalMetrics, nil
	case strings.HasSuffix(component, string(SignalLogs)):
		return SignalLogs, nil
	default:
		return "", fmt.Errorf("unknown signal type in component '%s'", component)
	}
}

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

// CreateAndConnectApp creates a SLIM app with shared secret authentication
// and connects it to a SLIM server.
//
// This is a convenience function that combines:
//   - Crypto initialization
//   - App creation with shared secret
//   - Server connection with TLS settings
//
// Args:
//
//	localID: Local identity string (org/namespace/app format)
//	serverAddr: SLIM server endpoint URL
//	secret: Shared secret for authentication (min 32 chars)
//
// Returns:
//
//	*slim.BindingsAdapter: Created and connected app instance
//	uint64: Connection ID returned by the server
//	error: If creation or connection fails
func CreateAndConnectApp(localID, serverAddr, secret string) (*slim.BindingsAdapter, uint64, error) {
	// Initialize crypto subsystem (idempotent, safe to call multiple times)
	slim.InitializeCryptoProvider()

	// Parse the local identity string
	appName, err := SplitID(localID)
	if err != nil {
		return nil, 0, fmt.Errorf("invalid local ID: %w", err)
	}

	// Create app with shared secret authentication
	app, err := slim.CreateAppWithSecret(appName, secret)
	if err != nil {
		return nil, 0, fmt.Errorf("create app failed: %w", err)
	}

	// Connect to SLIM server (returns connection ID)
	config := slim.ClientConfig{
		Endpoint: serverAddr,
		Tls:      slim.TlsConfig{Insecure: true},
	}
	connID, err := app.Connect(config)
	if err != nil {
		app.Destroy()
		return nil, 0, fmt.Errorf("connect failed: %w", err)
	}

	return app, connID, nil
}
