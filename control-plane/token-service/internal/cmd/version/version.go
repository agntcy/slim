// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package version

import (
	"fmt"
	"runtime"

	"github.com/spf13/cobra"
	"go.uber.org/zap"

	"github.com/agntcy/slim/control-plane/token-service/internal/options"
)

var (
	// Semantic version, output of git describe
	semVersion = "v0.0.0-master+$Format:%H$"

	// sha1 from git, output of $(git rev-parse HEAD).
	gitCommit = "$Format:%H$"

	// build date in ISO8601 format, output of $(date -u +'%Y-%m-%dT%H:%M:%SZ').
	buildDate = "1970-01-01T00:00:00Z"
)

type version struct {
	SemVersion string `json:"semVersion"`
	GitCommit  string `json:"gitCommit"`
	BuildDate  string `json:"buildDate"`
	GoVersion  string `json:"goVersion"`
	Compiler   string `json:"compiler"`
	Platform   string `json:"platform"`
}

func newVersion() version {
	// These variables usually come from -ldflags settings and in their
	// absence fallback to the ones defined in the var section.
	return version{
		SemVersion: semVersion,
		GitCommit:  gitCommit,
		BuildDate:  buildDate,
		GoVersion:  runtime.Version(),
		Compiler:   runtime.Compiler,
		Platform:   fmt.Sprintf("%s/%s", runtime.GOOS, runtime.GOARCH),
	}
}

// NewVersionCmd returns the version command.
func NewVersionCmd(opts *options.CommonOptions) *cobra.Command {
	v := newVersion()
	cmd := &cobra.Command{
		Use:           "version",
		Short:         "Print version information",
		Long:          "Print version information",
		Args:          cobra.NoArgs,
		SilenceErrors: true,
		SilenceUsage:  true,
		RunE: func(cmd *cobra.Command, _ []string) error {
			return Run(&v, opts, cmd)
		},
	}

	return cmd
}

// Run executes the business logic for the version command.
func Run(v *version, opts *options.CommonOptions, _ *cobra.Command) error {
	opts.Logger.Info(
		"Version",
		zap.String("SemVersion", v.SemVersion),
		zap.String("GitCommit", v.GitCommit),
		zap.String("BuildData", v.BuildDate),
		zap.String("GoVersion", v.GoVersion),
		zap.String("Compiler", v.Compiler),
		zap.String("Platform", v.Platform),
	)

	return nil
}
