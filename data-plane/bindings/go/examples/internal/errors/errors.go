// Package errors provides typed errors for SLIM example applications.
// These replace string-based error detection with proper error types.
package errors

import (
	"errors"
	"strings"
)

// Sentinel errors for SLIM operations.
var (
	// ErrTimeout indicates an operation timed out.
	ErrTimeout = errors.New("operation timed out")

	// ErrSessionNotFound indicates the requested session was not found.
	ErrSessionNotFound = errors.New("session not found")

	// ErrSessionClosed indicates the session has been closed.
	ErrSessionClosed = errors.New("session closed")

	// ErrParticipantDisconnected indicates a participant has disconnected.
	ErrParticipantDisconnected = errors.New("participant disconnected")

	// ErrInvitationNotFound indicates the requested invitation was not found.
	ErrInvitationNotFound = errors.New("invitation not found")

	// ErrInvitationExpired indicates the invitation has expired.
	ErrInvitationExpired = errors.New("invitation has expired")

	// ErrNotInitiator indicates the operation requires initiator privileges.
	ErrNotInitiator = errors.New("only session initiator can perform this action")

	// ErrInvalidDestination indicates an invalid destination format.
	ErrInvalidDestination = errors.New("invalid destination format (expected org/namespace/app)")

	// ErrNotGroupSession indicates the operation requires a group session.
	ErrNotGroupSession = errors.New("operation only valid for group sessions")

	// ErrAlreadyParticipant indicates the user is already a participant.
	ErrAlreadyParticipant = errors.New("user is already a participant")

	// ErrNotConnected indicates the SLIM connection is not established.
	ErrNotConnected = errors.New("not connected to SLIM server")
)

// WrapSLIMError converts raw SLIM binding errors into typed errors.
// This allows for proper error handling with errors.Is() instead of string matching.
func WrapSLIMError(err error) error {
	if err == nil {
		return nil
	}

	errStr := strings.ToLower(err.Error())

	switch {
	case strings.Contains(errStr, "timeout") || strings.Contains(errStr, "timed out"):
		return ErrTimeout
	case strings.Contains(errStr, "participant disconnected"):
		return ErrParticipantDisconnected
	case strings.Contains(errStr, "closed"):
		return ErrSessionClosed
	case strings.Contains(errStr, "not found"):
		return ErrSessionNotFound
	default:
		return err
	}
}

// IsTimeout checks if the error is a timeout error.
func IsTimeout(err error) bool {
	return errors.Is(err, ErrTimeout)
}

// IsSessionClosed checks if the error indicates a closed session.
func IsSessionClosed(err error) bool {
	return errors.Is(err, ErrSessionClosed)
}

// IsParticipantDisconnected checks if the error indicates a participant disconnected.
func IsParticipantDisconnected(err error) bool {
	return errors.Is(err, ErrParticipantDisconnected)
}

// IsRecoverable checks if the error is recoverable and the operation can be retried.
func IsRecoverable(err error) bool {
	return IsTimeout(err) || IsParticipantDisconnected(err)
}
