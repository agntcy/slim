package httputil

import (
	"context"
	"fmt"
	"log"
	"net/http"
	"time"
)

const (
	// DefaultShutdownTimeout is the default timeout for graceful shutdown.
	DefaultShutdownTimeout = 10 * time.Second
)

// Server wraps http.Server with graceful shutdown support.
type Server struct {
	httpServer      *http.Server
	shutdownTimeout time.Duration
}

// ServerOption is a function that configures a Server.
type ServerOption func(*Server)

// WithShutdownTimeout sets the shutdown timeout.
func WithShutdownTimeout(d time.Duration) ServerOption {
	return func(s *Server) {
		s.shutdownTimeout = d
	}
}

// NewServer creates a new Server.
func NewServer(addr string, handler http.Handler, opts ...ServerOption) *Server {
	s := &Server{
		httpServer: &http.Server{
			Addr:    addr,
			Handler: handler,
		},
		shutdownTimeout: DefaultShutdownTimeout,
	}
	for _, opt := range opts {
		opt(s)
	}
	return s
}

// Start starts the HTTP server and blocks until the context is cancelled.
// Returns an error if the server fails to start or shutdown.
func (s *Server) Start(ctx context.Context) error {
	errCh := make(chan error, 1)

	go func() {
		log.Printf("Starting HTTP server on %s", s.httpServer.Addr)
		if err := s.httpServer.ListenAndServe(); err != nil && err != http.ErrServerClosed {
			errCh <- fmt.Errorf("HTTP server error: %w", err)
		}
	}()

	select {
	case err := <-errCh:
		return err
	case <-ctx.Done():
		log.Println("Shutting down HTTP server...")
		return s.Shutdown()
	}
}

// Shutdown gracefully shuts down the server.
func (s *Server) Shutdown() error {
	ctx, cancel := context.WithTimeout(context.Background(), s.shutdownTimeout)
	defer cancel()

	if err := s.httpServer.Shutdown(ctx); err != nil {
		return fmt.Errorf("failed to shutdown HTTP server: %w", err)
	}
	log.Println("HTTP server shutdown complete")
	return nil
}

// Addr returns the server address.
func (s *Server) Addr() string {
	return s.httpServer.Addr
}
