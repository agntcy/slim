module github.com/agntcy/slim/bindings/go/examples/point_to_point

go 1.25.4

replace github.com/agntcy/slim/bindings/generated => ../../generated

replace github.com/agntcy/slim/bindings/go/examples/common => ../common

require (
	github.com/agntcy/slim/bindings/generated v0.0.0-00010101000000-000000000000
	github.com/agntcy/slim/bindings/go/examples/common v0.0.0-00010101000000-000000000000
)
