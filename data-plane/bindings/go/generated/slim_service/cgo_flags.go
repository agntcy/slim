package slim_service

// #cgo LDFLAGS: -L${SRCDIR}/../../../../target/release -lslim_service
// #cgo darwin LDFLAGS: -Wl,-rpath,${SRCDIR}/../../../../target/release
// #cgo linux LDFLAGS: -Wl,-rpath,${SRCDIR}/../../../../target/release
import "C"
