package util

import (
	"context"
	"os"
	"strings"
	"time"

	"github.com/rs/zerolog"

	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
)

func GetContextWithLogger(ctx context.Context, logConfig config.LogConfig) context.Context {
	logger := getDefaultContextLogger(logConfig.Level)
	return logger.WithContext(ctx)
}

func getDefaultContextLogger(logLevel string) zerolog.Logger {
	ll, err := zerolog.ParseLevel(strings.ToLower(logLevel))
	if err != nil {
		ll = zerolog.InfoLevel // Default to Info level if parsing fails
	}
	logger := zerolog.New(zerolog.ConsoleWriter{Out: os.Stdout, TimeFormat: time.RFC3339}).With().Timestamp().Logger()
	logger = logger.Level(ll)
	zerolog.DefaultContextLogger = &logger
	return logger
}
