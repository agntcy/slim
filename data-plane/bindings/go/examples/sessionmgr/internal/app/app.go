// Package app provides the application lifecycle for sessionmgr.
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
	"github.com/agntcy/slim/bindings/go/examples/sessionmgr/internal/handler"
	"github.com/agntcy/slim/bindings/go/examples/sessionmgr/internal/manager"
)

//go:embed templates/*.html templates/partials/*.html
var templateFS embed.FS

// App represents the sessionmgr application.
type App struct {
	config     *config.Config
	app        *slim.BindingsAdapter
	connID     uint64
	manager    *manager.Manager
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

	// Create manager
	mgr := manager.New(slimApp, connID, cfg.LocalID, globalSSE, sessionSSE)

	// Create auth handler
	auth := httputil.NewAuthHandler(cfg.CookieName("sessionmgr"))

	// Create handler
	h := handler.New(mgr, auth, renderer, globalSSE, sessionSSE)

	// Build routes
	mux := http.NewServeMux()
	mux.HandleFunc("/", h.Home)
	mux.HandleFunc("/login", auth.HandleLogin)
	mux.HandleFunc("/logout", auth.HandleLogout)
	mux.HandleFunc("/sessions", h.Sessions)
	mux.HandleFunc("/session/", h.SessionRoutes)
	mux.HandleFunc("/events", h.GlobalSSE)
	mux.HandleFunc("/clients", h.Clients)

	// Create server
	addr := fmt.Sprintf(":%d", cfg.HTTPPort)
	server := httputil.NewServer(addr, mux)

	return &App{
		config:     cfg,
		app:        slimApp,
		connID:     connID,
		manager:    mgr,
		server:     server,
		handler:    h,
		globalSSE:  globalSSE,
		sessionSSE: sessionSSE,
	}, nil
}

// Run starts the application and blocks until the context is cancelled.
func (a *App) Run(ctx context.Context) error {
	log.Printf("Starting SLIM Session Manager on http://localhost%s", a.server.Addr())
	log.Printf("SLIM server: %s, Local ID: %s", a.config.SlimEndpoint, a.config.LocalID)

	if a.manager.IsConnected() {
		// Start listening for incoming sessions
		go a.manager.ListenForIncomingSessions(ctx)
	}

	return a.server.Start(ctx)
}

// Close closes the application and releases resources.
func (a *App) Close() error {
	if a.app != nil {
		a.app.Destroy()
	}
	return nil
}
