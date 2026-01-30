module github.com/agntcy/slim/bindings/go

go 1.25

require github.com/agntcy/slim/bindings/generated v0.0.0

require google.golang.org/protobuf v1.36.11 // indirect

replace github.com/agntcy/slim/bindings/generated => ./generated
