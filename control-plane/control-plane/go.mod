module github.com/agntcy/slim/control-plane/control-plane

go 1.25.7

require (
	github.com/agntcy/slim/control-plane/common v0.0.0-00010101000000-000000000000
	github.com/google/uuid v1.6.0
	github.com/rs/zerolog v1.34.0
	github.com/spiffe/go-spiffe/v2 v2.5.0
	github.com/stretchr/testify v1.10.0
	google.golang.org/grpc v1.73.0
	google.golang.org/protobuf v1.36.6
	gopkg.in/yaml.v3 v3.0.1
	k8s.io/client-go v0.34.1
)

require (
	github.com/glebarez/sqlite v1.11.0
	gorm.io/gorm v1.25.7
)

require (
	github.com/Microsoft/go-winio v0.6.2 // indirect
	github.com/davecgh/go-spew v1.1.1 // indirect
	github.com/dustin/go-humanize v1.0.1 // indirect
	github.com/glebarez/go-sqlite v1.22.0 // indirect
	github.com/go-jose/go-jose/v4 v4.0.5 // indirect
	github.com/go-logr/logr v1.4.2 // indirect
	github.com/jinzhu/inflection v1.0.0 // indirect
	github.com/jinzhu/now v1.1.5 // indirect
	github.com/kr/text v0.2.0 // indirect
	github.com/mattn/go-colorable v0.1.13 // indirect
	github.com/mattn/go-isatty v0.0.20 // indirect
	github.com/pmezard/go-difflib v1.0.0 // indirect
	github.com/remyoudompheng/bigfft v0.0.0-20230129092748-24d4a6f8daec // indirect
	github.com/stretchr/objx v0.5.2 // indirect
	github.com/zeebo/errs v1.4.0 // indirect
	golang.org/x/crypto v0.45.0 // indirect
	golang.org/x/net v0.47.0 // indirect
	golang.org/x/sys v0.38.0 // indirect
	golang.org/x/text v0.31.0 // indirect
	golang.org/x/time v0.9.0 // indirect
	google.golang.org/genproto/googleapis/rpc v0.0.0-20250324211829-b45e905df463 // indirect
	k8s.io/apimachinery v0.34.1 // indirect
	k8s.io/klog/v2 v2.130.1 // indirect
	k8s.io/utils v0.0.0-20250604170112-4c0f3b243397 // indirect
	modernc.org/libc v1.37.6 // indirect
	modernc.org/mathutil v1.6.0 // indirect
	modernc.org/memory v1.7.2 // indirect
	modernc.org/sqlite v1.28.0 // indirect
)

replace github.com/agntcy/slim/control-plane/common => ../common
