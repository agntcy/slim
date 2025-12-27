package template

import (
	"embed"
	"fmt"
	"html/template"
	"io"
	"net/http"
)

// Renderer handles template rendering with go:embed support.
type Renderer struct {
	templates *template.Template
}

// NewRenderer creates a new template renderer from an embedded filesystem.
// The pattern should match the template files, e.g., "templates/*.html".
func NewRenderer(fs embed.FS, patterns ...string) (*Renderer, error) {
	tmpl := template.New("").Funcs(DefaultFuncs())

	for _, pattern := range patterns {
		var err error
		tmpl, err = tmpl.ParseFS(fs, pattern)
		if err != nil {
			return nil, fmt.Errorf("failed to parse templates: %w", err)
		}
	}

	return &Renderer{templates: tmpl}, nil
}

// NewRendererWithFuncs creates a new template renderer with custom functions.
func NewRendererWithFuncs(fs embed.FS, funcs template.FuncMap, patterns ...string) (*Renderer, error) {
	tmpl := template.New("").Funcs(funcs)

	for _, pattern := range patterns {
		var err error
		tmpl, err = tmpl.ParseFS(fs, pattern)
		if err != nil {
			return nil, fmt.Errorf("failed to parse templates: %w", err)
		}
	}

	return &Renderer{templates: tmpl}, nil
}

// Render executes a template by name and writes the output to the writer.
func (r *Renderer) Render(w io.Writer, name string, data interface{}) error {
	return r.templates.ExecuteTemplate(w, name, data)
}

// RenderHTTP is a convenience method that renders a template and writes to an http.ResponseWriter.
// It sets the Content-Type header to text/html and handles errors by returning HTTP 500.
func (r *Renderer) RenderHTTP(w http.ResponseWriter, name string, data interface{}) {
	w.Header().Set("Content-Type", "text/html; charset=utf-8")
	if err := r.Render(w, name, data); err != nil {
		http.Error(w, fmt.Sprintf("Template error: %v", err), http.StatusInternalServerError)
	}
}

// RenderPartial renders a template without setting headers (useful for HTMX responses).
func (r *Renderer) RenderPartial(w http.ResponseWriter, name string, data interface{}) {
	if err := r.Render(w, name, data); err != nil {
		http.Error(w, fmt.Sprintf("Template error: %v", err), http.StatusInternalServerError)
	}
}

// Templates returns the underlying template.Template for advanced use cases.
func (r *Renderer) Templates() *template.Template {
	return r.templates
}

// MustNewRenderer creates a new renderer and panics on error.
// This is useful for package-level initialization.
func MustNewRenderer(fs embed.FS, patterns ...string) *Renderer {
	r, err := NewRenderer(fs, patterns...)
	if err != nil {
		panic(err)
	}
	return r
}
