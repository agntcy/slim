package manager

import (
	"context"

	"go.uber.org/zap"
)

// Manager defines management operations for a local slim instance.
type Manager interface {
	Start(ctx context.Context) error
	Stop(ctx context.Context) error
	Status(ctx context.Context) (string, error)
}

// Service is the default implementation of Manager.
type manager struct {
	Logger *zap.Logger
}

// NewService creates a new Service.
func NewManager(logger *zap.Logger) Manager {
	return &manager{Logger: logger}
}

// Start starts the local slim instance.
func (s *manager) Start(ctx context.Context) error {
	if s.Logger != nil {
		s.Logger.Info("Starting slim instance")
	}
	return nil
}

// Stop stops the local slim instance.
func (s *manager) Stop(ctx context.Context) error {
	if s.Logger != nil {
		s.Logger.Info("Stopping slim instance")
	}
	return nil
}

// Status returns the status of the local slim instance.
func (s *manager) Status(ctx context.Context) (string, error) {
	if s.Logger != nil {
		s.Logger.Info("Getting status of slim instance")
	}
	return "unknown", nil
}
