// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package logging

import (
	"go.uber.org/zap"
	"go.uber.org/zap/zapcore"
)

const (
	// Logging levels

	// DebugLevel logs are typically voluminous, and are usually disabled in
	// production.
	WheelDebug = "debug"

	// InfoLevel is the default logging priority.
	WheelInfo = "info"

	// WarnLevel logs are more important than Info, but don't need individual
	// human review.
	WheelWarning = "warning"

	// ErrorLevel logs are high-priority. If an application is running smoothly,
	// it shouldn't generate any error-level logs.
	WheelError = "error"

	// DPanicLevel logs are particularly important errors. In development the
	// logger panics after writing the message.
	WheelDPanic = "dpanic"

	// PanicLevel logs a message, then panics.
	WheelPanic = "panic"

	// FatalLevel logs a message, then calls os.Exit(1).
	WheelFatal = "fatal"
)

// Function to map logging level string to zap logging level
func GetZapLogLevel(level string) zapcore.Level {
	switch level {
	case WheelDebug:
		return zap.DebugLevel
	case WheelInfo:
		return zap.InfoLevel
	case WheelWarning:
		return zap.WarnLevel
	case WheelError:
		return zap.ErrorLevel
	case WheelDPanic:
		return zap.DPanicLevel
	case WheelPanic:
		return zap.PanicLevel
	case WheelFatal:
		return zap.FatalLevel
	default:
		return zap.InfoLevel
	}
}
