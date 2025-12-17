module github.com/agntcy/slim/otel

go 1.25.5

require (
	github.com/agntcy/slim/bindings/generated v0.0.0
)

replace github.com/agntcy/slim/bindings/generated => ../data-plane/bindings/go/generated/

