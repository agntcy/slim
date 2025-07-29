module github.com/agntcy/slim/control-plane/slimctl

go 1.23.6

require (
	github.com/agntcy/slim/control-plane/common v0.0.0-00010101000000-000000000000
	github.com/google/uuid v1.6.0
	github.com/knadh/koanf v1.5.0
	github.com/spf13/cobra v1.8.1
	github.com/spf13/pflag v1.0.6
	go.uber.org/zap v1.27.0
	google.golang.org/protobuf v1.36.6
)

require (
	github.com/fsnotify/fsnotify v1.8.0 // indirect
	github.com/inconshreveable/mousetrap v1.1.0 // indirect
	github.com/kr/pretty v0.3.1 // indirect
	github.com/mitchellh/copystructure v1.2.0 // indirect
	github.com/mitchellh/mapstructure v1.5.0 // indirect
	github.com/mitchellh/reflectwalk v1.0.2 // indirect
	github.com/stretchr/testify v1.10.0 // indirect
	github.com/xeipuuv/gojsonpointer v0.0.0-20180127040702-4e3ac2762d5f // indirect
	github.com/xeipuuv/gojsonreference v0.0.0-20180127040603-bd5ef7bd5415 // indirect
	github.com/xeipuuv/gojsonschema v1.2.0 // indirect
	go.uber.org/multierr v1.10.0 // indirect
	golang.org/x/net v0.38.0 // indirect
	golang.org/x/sys v0.31.0 // indirect
	golang.org/x/text v0.23.0 // indirect
	google.golang.org/genproto/googleapis/rpc v0.0.0-20250324211829-b45e905df463 // indirect
	google.golang.org/grpc v1.73.0 // indirect
	gopkg.in/yaml.v3 v3.0.1 // indirect
)

replace github.com/agntcy/slim/control-plane/common => ../common
