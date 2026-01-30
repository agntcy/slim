module github.com/agntcy/slim/bindings/go/examples/slimrpc/simple

go 1.25

require (
	github.com/agntcy/slim/bindings/generated v0.0.0
	google.golang.org/protobuf v1.36.1
)

replace github.com/agntcy/slim/bindings/generated => ../../../generated
