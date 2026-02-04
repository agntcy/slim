// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

//go:build tools
// +build tools

package tools

// https://go.dev/wiki/Modules#how-can-i-track-tool-dependencies-for-a-module

import (
	_ "github.com/golangci/golangci-lint/cmd/golangci-lint"
	_ "github.com/ory/go-acc"
	_ "github.com/pavius/impi/cmd/impi"
	_ "go.opentelemetry.io/build-tools/multimod"
	_ "golang.org/x/vuln/cmd/govulncheck"
	_ "google.golang.org/grpc/cmd/protoc-gen-go-grpc"
	_ "google.golang.org/protobuf/cmd/protoc-gen-go"
)
