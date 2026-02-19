// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

// Package common provides shared helper utilities for SLIM Go binding examples.
//
// This package provides:
//   - Identity string parsing (org/namespace/app)
//   - App creation and connection helper
//   - Default configuration values
package common

import (
	slim "github.com/agntcy/slim-bindings-go"
)

// Default configuration values
const (
	DefaultServerEndpoint = "http://localhost:46357"
	DefaultSharedSecret   = "demo-shared-secret-min-32-chars!!"
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
func SplitID(id string) (*slim.Name, error) {
	return slim.NewNameFromString(id)
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
func CreateAndConnectApp(localID, serverAddr, secret string) (*slim.App, uint64, error) {
	// Initialize crypto, runtime, global service and logging with defaults
	slim.InitializeWithDefaults()

	// Parse the local identity string
	appName, err := SplitID(localID)
	if err != nil {
		return nil, 0, fmt.Errorf("invalid local ID: %w", err)
	}

	// Create app with shared secret authentication
	app, err := slim.GetGlobalService().CreateAppWithSecret(appName, secret)
	if err != nil {
		return nil, 0, fmt.Errorf("create app failed: %w", err)
	}

	// Connect to SLIM server (returns connection ID)
	config := slim.NewInsecureClientConfig(serverAddr)
	connID, err := slim.GetGlobalService().ConnectAsync(config)
	if err != nil {
		app.Destroy()
		return nil, 0, fmt.Errorf("connect failed: %w", err)
	}

	// Forward subscription to next node
	err = app.SubscribeAsync(app.Name(), &connID)
	if err != nil {
		app.Destroy()
		return nil, 0, fmt.Errorf("subscribe failed: %w", err)
	}

	return app, connID, nil
}
