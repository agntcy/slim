package session

import (
	"context"
	"log"
	"sync"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
	"github.com/agntcy/slim/bindings/go/examples/internal/config"
	slimErrors "github.com/agntcy/slim/bindings/go/examples/internal/errors"
)

// MessageHandler is called when a message is received.
type MessageHandler func(msg slim.ReceivedMessage)

// StatusHandler is called when session status changes.
type StatusHandler func(status Status)

// ReceiverConfig holds configuration for the message receiver.
type ReceiverConfig struct {
	// TimeoutMs is the timeout for GetMessage in milliseconds.
	TimeoutMs uint32

	// OnMessage is called when a message is received.
	OnMessage MessageHandler

	// OnStatusChange is called when the session status changes.
	OnStatusChange StatusHandler
}

// DefaultReceiverConfig returns a ReceiverConfig with default values.
func DefaultReceiverConfig() *ReceiverConfig {
	return &ReceiverConfig{
		TimeoutMs:      uint32(config.GetMessageTimeoutMs),
		OnMessage:      func(msg slim.ReceivedMessage) {},
		OnStatusChange: func(status Status) {},
	}
}

// StartReceiver starts a message receiver goroutine for a session.
// Returns a cancel function to stop the receiver.
func StartReceiver(
	ctx context.Context,
	session *slim.BindingsSessionContext,
	mu *sync.Mutex,
	cfg *ReceiverConfig,
) context.CancelFunc {
	if cfg == nil {
		cfg = DefaultReceiverConfig()
	}

	ctx, cancel := context.WithCancel(ctx)

	go func() {
		for {
			select {
			case <-ctx.Done():
				return
			default:
			}

			timeout := cfg.TimeoutMs

			mu.Lock()
			msg, err := session.GetMessage(&timeout)
			mu.Unlock()

			if err != nil {
				wrappedErr := slimErrors.WrapSLIMError(err)

				if slimErrors.IsTimeout(wrappedErr) {
					// Timeout is normal, continue polling
					continue
				}

				if slimErrors.IsParticipantDisconnected(wrappedErr) {
					// Participant disconnected is informational
					log.Printf("Participant disconnected: %v", err)
					continue
				}

				// Session closed or fatal error
				log.Printf("Session error, stopping receiver: %v", err)
				cfg.OnStatusChange(StatusClosed)
				return
			}

			cfg.OnMessage(msg)
		}
	}()

	return cancel
}

// StartReceiverWithStopChan starts a message receiver that also listens to a stop channel.
// This is useful when the session has a stopChan field that can be used to signal shutdown.
func StartReceiverWithStopChan(
	ctx context.Context,
	session *slim.BindingsSessionContext,
	mu *sync.Mutex,
	stopChan <-chan struct{},
	cfg *ReceiverConfig,
) context.CancelFunc {
	if cfg == nil {
		cfg = DefaultReceiverConfig()
	}

	ctx, cancel := context.WithCancel(ctx)

	go func() {
		for {
			select {
			case <-ctx.Done():
				return
			case <-stopChan:
				return
			default:
			}

			timeout := cfg.TimeoutMs

			mu.Lock()
			msg, err := session.GetMessage(&timeout)
			mu.Unlock()

			if err != nil {
				wrappedErr := slimErrors.WrapSLIMError(err)

				if slimErrors.IsTimeout(wrappedErr) {
					continue
				}

				if slimErrors.IsParticipantDisconnected(wrappedErr) {
					log.Printf("Participant disconnected: %v", err)
					continue
				}

				log.Printf("Session error, stopping receiver: %v", err)
				cfg.OnStatusChange(StatusClosed)
				return
			}

			cfg.OnMessage(msg)
		}
	}()

	return cancel
}
