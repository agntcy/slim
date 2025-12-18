module github.com/agntcy/slim/otel

go 1.25.5

require github.com/agntcy/slim/bindings/generated v0.0.0

require (
	github.com/hashicorp/go-version v1.8.0 // indirect
	github.com/json-iterator/go v1.1.12 // indirect
	github.com/modern-go/concurrent v0.0.0-20180306012644-bacd9c7ef1dd // indirect
	github.com/modern-go/reflect2 v1.0.3-0.20250322232337-35a7c28c31ee // indirect
	go.opentelemetry.io/collector/featuregate v1.48.0 // indirect
	go.opentelemetry.io/collector/pdata v1.48.0
	go.uber.org/multierr v1.11.0 // indirect
)

replace github.com/agntcy/slim/bindings/generated => ../data-plane/bindings/go/generated/
