// Package template provides template rendering utilities for SLIM example applications.
package template

import (
	"encoding/json"
	"html/template"
	"time"
)

// DefaultFuncs returns the default template functions used by SLIM example applications.
func DefaultFuncs() template.FuncMap {
	return template.FuncMap{
		"json":       toJSON,
		"formatTime": formatTime,
		"formatDate": formatDate,
		"add":        add,
		"sub":        sub,
	}
}

// toJSON converts a value to a JSON string.
func toJSON(v interface{}) string {
	b, err := json.Marshal(v)
	if err != nil {
		return "{}"
	}
	return string(b)
}

// formatTime formats a time.Time as HH:MM:SS.
func formatTime(t time.Time) string {
	return t.Format("15:04:05")
}

// formatDate formats a time.Time as YYYY-MM-DD HH:MM:SS.
func formatDate(t time.Time) string {
	return t.Format("2006-01-02 15:04:05")
}

// add adds two integers.
func add(a, b int) int {
	return a + b
}

// sub subtracts b from a.
func sub(a, b int) int {
	return a - b
}

// FormatMessageHTML formats a message for HTML display with proper escaping.
func FormatMessageHTML(direction, sender, text string, timestamp time.Time) string {
	escapedSender := template.HTMLEscapeString(sender)
	escapedText := template.HTMLEscapeString(text)
	formattedTime := timestamp.Format("15:04:05")

	if direction == "sent" {
		return `<div class="message sent">` +
			`<span class="sender">You</span>` +
			`<span class="text">` + escapedText + `</span>` +
			`<span class="time">` + formattedTime + `</span>` +
			`</div>`
	}
	return `<div class="message received">` +
		`<span class="sender">` + escapedSender + `</span>` +
		`<span class="text">` + escapedText + `</span>` +
		`<span class="time">` + formattedTime + `</span>` +
		`</div>`
}
