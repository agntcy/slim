// Package httputil provides HTTP utilities for SLIM example applications.
package httputil

import (
	"net/http"
	"strings"

	"github.com/agntcy/slim/bindings/go/examples/internal/config"
)

// AuthHandler handles cookie-based authentication.
type AuthHandler struct {
	cookieName string
	homePath   string
}

// NewAuthHandler creates a new AuthHandler with the given cookie name.
func NewAuthHandler(cookieName string) *AuthHandler {
	return &AuthHandler{
		cookieName: cookieName,
		homePath:   "/",
	}
}

// GetUsername returns the username from the request cookie, or empty string if not set.
func (h *AuthHandler) GetUsername(r *http.Request) string {
	cookie, err := r.Cookie(h.cookieName)
	if err != nil {
		return ""
	}
	return cookie.Value
}

// HandleLogin handles the login POST request.
func (h *AuthHandler) HandleLogin(w http.ResponseWriter, r *http.Request) {
	if r.Method != http.MethodPost {
		http.Redirect(w, r, h.homePath, http.StatusSeeOther)
		return
	}

	username := strings.TrimSpace(r.FormValue("username"))
	if username == "" {
		http.Error(w, "Username is required", http.StatusBadRequest)
		return
	}

	http.SetCookie(w, &http.Cookie{
		Name:   h.cookieName,
		Value:  username,
		Path:   "/",
		MaxAge: config.CookieMaxAge,
	})
	http.Redirect(w, r, h.homePath, http.StatusSeeOther)
}

// HandleLogout handles the logout request.
func (h *AuthHandler) HandleLogout(w http.ResponseWriter, r *http.Request) {
	http.SetCookie(w, &http.Cookie{
		Name:   h.cookieName,
		Value:  "",
		Path:   "/",
		MaxAge: -1,
	})
	http.Redirect(w, r, h.homePath, http.StatusSeeOther)
}

// RequireAuth is middleware that checks for authentication.
// If the user is not authenticated, they are redirected to the home page.
func (h *AuthHandler) RequireAuth(next http.HandlerFunc) http.HandlerFunc {
	return func(w http.ResponseWriter, r *http.Request) {
		if h.GetUsername(r) == "" {
			http.Redirect(w, r, h.homePath, http.StatusSeeOther)
			return
		}
		next(w, r)
	}
}
