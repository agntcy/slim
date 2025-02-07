module github.com/agntcy/agp/control-plane/token-service

replace github.com/agntcy/agp/control-plane => ..

go 1.23.6

require (
	github.com/agntcy/agp/control-plane v0.0.0-00010101000000-000000000000
	github.com/spf13/cobra v1.8.1
	go.uber.org/zap v1.27.0
)

require (
	github.com/inconshreveable/mousetrap v1.1.0 // indirect
	github.com/spf13/pflag v1.0.5 // indirect
	go.uber.org/multierr v1.10.0 // indirect
)
