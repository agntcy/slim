module github.com/agntcy/slim/otel

go 1.25.5

require github.com/agntcy/slim/bindings/generated v0.0.0

require github.com/stretchr/testify v1.11.1 // indirect

require (
	go.uber.org/multierr v1.11.0 // indirect
	go.uber.org/zap v1.27.1
)

replace github.com/agntcy/slim/bindings/generated => ../data-plane/bindings/go/generated/
