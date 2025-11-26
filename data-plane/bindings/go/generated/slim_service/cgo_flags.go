package slim_service

// #cgo LDFLAGS: -L/Users/micpapal/Documents/code/agntcy/slim/data-plane/target/release -lslim_service
// #cgo darwin LDFLAGS: -Wl,-rpath,/Users/micpapal/Documents/code/agntcy/slim/data-plane/target/release
// #cgo linux LDFLAGS: -Wl,-rpath,/Users/micpapal/Documents/code/agntcy/slim/data-plane/target/release
import "C"
