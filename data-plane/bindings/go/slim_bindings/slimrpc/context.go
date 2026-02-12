package slimrpc

import (
	"context"

	slim_bindings "github.com/agntcy/slim-bindings-go"
)

type contextKey int

const (
	sessionContextKey contextKey = iota
	metadataContextKey
)

// WithMetadata returns a new context with the given metadata attached
func WithMetadata(ctx context.Context, metadata map[string]string) context.Context {
	return context.WithValue(ctx, metadataContextKey, metadata)
}

// MetadataFromContext extracts the metadata from the context
// Returns the metadata map and true if found, or nil and false if not found
func MetadataFromContext(ctx context.Context) (map[string]string, bool) {
	metadata, ok := ctx.Value(metadataContextKey).(map[string]string)
	return metadata, ok
}

// WithSessionId returns a new context with the given session ID attached
func WithSessionId(ctx context.Context, sessionId string) context.Context {
	return context.WithValue(ctx, sessionContextKey, sessionId)
}

// SessionIdFromContext extracts the session ID from the context
// Returns the session ID and true if found, or empty string and false if not found
func SessionIdFromContext(ctx context.Context) (string, bool) {
	sessionId, ok := ctx.Value(sessionContextKey).(string)
	return sessionId, ok
}

// ContextFromRpcContext creates a Go context.Context from a slim_bindings.Context
// It extracts the deadline, metadata, and session ID and applies them to the context
func ContextFromRpcContext(rpcContext *slim_bindings.Context) (context.Context, context.CancelFunc) {
	ctx := context.Background()

	// Get deadline and create context with timeout/deadline
	deadline := rpcContext.Deadline()
	var cancel context.CancelFunc
	if !deadline.IsZero() {
		ctx, cancel = context.WithDeadline(ctx, deadline)
	} else {
		// Create a no-op cancel function if no deadline
		cancel = func() {}
	}

	// Add session ID to context
	sessionId := rpcContext.SessionId()
	if sessionId != "" {
		ctx = WithSessionId(ctx, sessionId)
	}

	// Add metadata to context
	metadata := rpcContext.Metadata()
	if len(metadata) > 0 {
		ctx = WithMetadata(ctx, metadata)
	}

	return ctx, cancel
}

// ContextWithTimeout creates a Go context.Context with a timeout based on RemainingTime
// This is useful when you want to use the remaining time as a timeout instead of an absolute deadline
func ContextWithTimeout(rpcContext *slim_bindings.Context) (context.Context, context.CancelFunc) {
	ctx := context.Background()

	// Get remaining time and create context with timeout
	remainingTime := rpcContext.RemainingTime()
	var cancel context.CancelFunc
	if remainingTime > 0 {
		ctx, cancel = context.WithTimeout(ctx, remainingTime)
	} else {
		// Create a no-op cancel function if no time remaining
		cancel = func() {}
	}

	// Add session ID to context
	sessionId := rpcContext.SessionId()
	if sessionId != "" {
		ctx = WithSessionId(ctx, sessionId)
	}

	// Add metadata to context
	metadata := rpcContext.Metadata()
	if len(metadata) > 0 {
		ctx = WithMetadata(ctx, metadata)
	}

	return ctx, cancel
}
