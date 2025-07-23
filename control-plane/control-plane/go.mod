module github.com/agntcy/slim/control-plane/control-plane

go 1.23.6

require (
	github.com/agntcy/slim/control-plane/common v0.0.0-00010101000000-000000000000
	github.com/google/uuid v1.6.0
	github.com/rs/zerolog v1.34.0
	google.golang.org/grpc v1.73.0
	google.golang.org/protobuf v1.36.6
	gopkg.in/yaml.v3 v3.0.1
)

require (
	github.com/mattn/go-colorable v0.1.13 // indirect
	github.com/mattn/go-isatty v0.0.19 // indirect
	go.uber.org/multierr v1.10.0 // indirect
	go.uber.org/zap v1.27.0 // indirect
	golang.org/x/net v0.38.0 // indirect
	golang.org/x/sys v0.31.0 // indirect
	golang.org/x/text v0.23.0 // indirect
	google.golang.org/genproto/googleapis/rpc v0.0.0-20250324211829-b45e905df463 // indirect
)

replace github.com/agntcy/slim/control-plane/common => ../common
