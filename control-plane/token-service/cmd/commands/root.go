// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package commands

import (
	"context"

	"github.com/spf13/cobra"
	"go.uber.org/zap"
	"go.uber.org/zap/zapcore"

	"github.com/agntcy/slim/control-plane/token-service/cmd/common"
	"github.com/agntcy/slim/control-plane/token-service/internal/cmd/version"
	"github.com/agntcy/slim/control-plane/token-service/internal/logging"
	"github.com/agntcy/slim/control-plane/token-service/internal/options"
)

// New instantiates the root command and initializes the tree of commands.
func New(opts *options.CommonOptions) (*cobra.Command, error) {
	rootOpts := &common.RootOptions{
		CommonOptions: opts,
	}

	rootCmd := &cobra.Command{
		Use:               "token-service",
		Short:             "SLIM token service",
		DisableAutoGenTag: true,
		PersistentPreRunE: func(cmd *cobra.Command, _ []string) error {
			// Setup logging config
			return setupLogging(cmd, opts)
		},
	}

	// Commands
	rootCmd.AddCommand(version.NewVersionCmd(opts))

	// Log settings
	rootCmd.PersistentFlags().StringVarP(
		&opts.LogLevel,
		"log-level",
		"l",
		"info",
		"Log level. Valid values are: "+
			"debug, info, warn, error, dpanic, panic, fatal.",
	)

	rootCmd.PersistentFlags().StringVarP(
		&rootOpts.LogFile,
		"log-output-path",
		"o",
		"stdout",
		"Output path for log",
	)

	_ = rootCmd.MarkPersistentFlagRequired("token")

	// return
	return rootCmd, nil
}

// Execute configures the signal handlers and runs the command.
func Execute(
	ctx context.Context,
	cmd *cobra.Command,
	opts *options.CommonOptions,
) error {
	// we do not log the error here since we expect that each subcommand
	// handles the errors by itself.
	err := cmd.ExecuteContext(ctx)
	if err != nil {
		opts.Logger.Error("error", zap.Error(err))
	}

	return err
}

func setupLogging(cmd *cobra.Command, opts *options.CommonOptions) error {
	// Get log output path
	output, err := cmd.Flags().GetString("log-output-path")
	if err != nil {
		return err
	}

	logLevel, err := cmd.Flags().GetString("log-level")
	if err != nil {
		return err
	}

	encoderConfig := zapcore.EncoderConfig{
		MessageKey:     "message",
		LevelKey:       "level",
		TimeKey:        "time",
		NameKey:        "logger",
		EncodeLevel:    zapcore.CapitalColorLevelEncoder, // Use color-coded levels
		EncodeTime:     zapcore.ISO8601TimeEncoder,       // Use ISO8601 time format
		EncodeDuration: zapcore.StringDurationEncoder,
		EncodeCaller:   zapcore.ShortCallerEncoder,
	}

	zapConfig := zap.NewProductionConfig()
	// zapConfig.DisableStacktrace = true
	zapConfig.OutputPaths = []string{output}
	zapConfig.Level = zap.NewAtomicLevelAt(logging.GetZapLogLevel(logLevel))
	zapConfig.Encoding = "console"
	zapConfig.EncoderConfig = encoderConfig
	opts.Logger, err = zapConfig.Build()
	if err != nil {
		return err
	}

	return nil
}
