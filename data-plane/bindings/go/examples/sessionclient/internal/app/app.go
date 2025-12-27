// Package app provides the application lifecycle for sessionclient.
package app

import (
	"context"
	"embed"
	"fmt"
	"log"
	"net/http"

	slim "github.com/agntcy/slim/bindings/generated/slim_bindings"
	"github.com/agntcy/slim/bindings/go/examples/common"
	"github.com/agntcy/slim/bindings/go/examples/internal/config"
	"github.com/agntcy/slim/bindings/go/examples/internal/httputil"
	tmpl "github.com/agntcy/slim/bindings/go/examples/internal/template"
	"github.com/agntcy/slim/bindings/go/examples/sessionclient/internal/client"
	"github.com/agntcy/slim/bindings/go/examples/sessionclient/internal/handler"
)

//go:embed templates/*.html templates/partials/*.html
var templateFS embed.FS

// App represents the sessionclient application.
type App struct {
	config     *config.Config
	app        *slim.BindingsAdapter
	connID     uint64
	client     *client.Client
	server     *httputil.Server
	handler    *handler.Handler
	globalSSE  httputil.Broadcaster
	sessionSSE httputil.SessionBroadcaster
}

// New creates a new App.
func New(cfg *config.Config) (*App, error) {
	// Parse templates
	renderer, err := tmpl.NewRenderer(templateFS,
		"templates/*.html",
		"templates/partials/*.html",
	)
	if err != nil {
		return nil, fmt.Errorf("failed to parse templates: %w", err)
	}

	// Initialize SSE broadcasters
	globalSSE := httputil.NewBroadcaster(config.DefaultSSEBufferSize)
	sessionSSE := httputil.NewSessionBroadcaster(config.DefaultSSEBufferSize)

	// Initialize SLIM connection
	var slimApp *slim.BindingsAdapter
	var connID uint64

	slimApp, connID, err = common.CreateAndConnectApp(cfg.LocalID, cfg.SlimEndpoint, cfg.SharedSecret)
	if err != nil {
		log.Printf("Warning: SLIM connection failed: %v", err)
		log.Printf("Running in demo mode without SLIM connectivity")
		slimApp = nil
		connID = 0
	}

	// Create client
	c := client.New(slimApp, connID, cfg.LocalID, cfg.InviteTimeout, globalSSE, sessionSSE)

	// Create auth handler
	auth := httputil.NewAuthHandler(cfg.CookieName("sessionclient"))

	// Create handler
	h := handler.New(c, auth, renderer, globalSSE, sessionSSE)

	// Build routes
	mux := http.NewServeMux()
	mux.HandleFunc("/", h.Home)
	mux.HandleFunc("/login", auth.HandleLogin)
	mux.HandleFunc("/logout", auth.HandleLogout)
	mux.HandleFunc("/invitations", h.Invitations)
	mux.HandleFunc("/invitation/", h.InvitationRoutes)
	mux.HandleFunc("/sessions", h.Sessions)
	mux.HandleFunc("/session/", h.SessionRoutes)
	mux.HandleFunc("/events", h.GlobalSSE)

	// Create server
	addr := fmt.Sprintf(":%d", cfg.HTTPPort)
	server := httputil.NewServer(addr, mux)

	return &App{
		config:     cfg,
		app:        slimApp,
		connID:     connID,
		client:     c,
		server:     server,
		handler:    h,
		globalSSE:  globalSSE,
		sessionSSE: sessionSSE,
	}, nil
}

// Run starts the application and blocks until the context is cancelled.
func (a *App) Run(ctx context.Context) error {
	log.Printf("Starting SLIM Session Client on http://localhost%s", a.server.Addr())
	log.Printf("SLIM server: %s, Local ID: %s", a.config.SlimEndpoint, a.config.LocalID)
	log.Printf("Invitation timeout: %d seconds", a.config.InviteTimeout)

	if a.client.IsConnected() {
		// Start listening for invitations
		go a.client.ListenForInvitations(ctx)
		// Start cleanup goroutine
		go a.client.CleanupExpiredInvitations(ctx)
	}

	return a.server.Start(ctx)
}

// Close closes the application and releases resources.
func (a *App) Close() error {
	a.client.Close()
	if a.app != nil {
		a.app.Destroy()
	}
	return nil
}
