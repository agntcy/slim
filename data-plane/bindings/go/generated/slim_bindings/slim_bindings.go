package slim_bindings

/*
#cgo CFLAGS: -I${SRCDIR}
#cgo linux,amd64 LDFLAGS: -L${SRCDIR} -L${SRCDIR}/../../../../../../.cache/slim-bindings -lslim_bindings_x86_64_linux_gnu -lm
#cgo linux,arm64 LDFLAGS: -L${SRCDIR} -L${SRCDIR}/../../../../../../.cache/slim-bindings -lslim_bindings_aarch64_linux_gnu -lm
#cgo darwin,amd64 LDFLAGS: -L${SRCDIR} -L${SRCDIR}/../../../../../../.cache/slim-bindings -lslim_bindings_x86_64_darwin -Wl,-undefined,dynamic_lookup
#cgo darwin,arm64 LDFLAGS: -L${SRCDIR} -L${SRCDIR}/../../../../../../.cache/slim-bindings -lslim_bindings_aarch64_darwin -Wl,-undefined,dynamic_lookup
#cgo windows,amd64 LDFLAGS: -L${SRCDIR} -L${SRCDIR}/../../../../../../AppData/Local/slim-bindings -lslim_bindings_x86_64_windows_gnu -lws2_32 -lbcrypt -ladvapi32 -luserenv -lntdll -lgcc_eh -lgcc -lkernel32 -lole32
#include <slim_bindings.h>
*/
import "C"

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"io"
	"math"
	"runtime"
	"runtime/cgo"
	"sync/atomic"
	"time"
	"unsafe"
)

// This is needed, because as of go 1.24
// type RustBuffer C.RustBuffer cannot have methods,
// RustBuffer is treated as non-local type
type GoRustBuffer struct {
	inner C.RustBuffer
}

type RustBufferI interface {
	AsReader() *bytes.Reader
	Free()
	ToGoBytes() []byte
	Data() unsafe.Pointer
	Len() uint64
	Capacity() uint64
}

func RustBufferFromExternal(b RustBufferI) GoRustBuffer {
	return GoRustBuffer{
		inner: C.RustBuffer{
			capacity: C.uint64_t(b.Capacity()),
			len:      C.uint64_t(b.Len()),
			data:     (*C.uchar)(b.Data()),
		},
	}
}

func (cb GoRustBuffer) Capacity() uint64 {
	return uint64(cb.inner.capacity)
}

func (cb GoRustBuffer) Len() uint64 {
	return uint64(cb.inner.len)
}

func (cb GoRustBuffer) Data() unsafe.Pointer {
	return unsafe.Pointer(cb.inner.data)
}

func (cb GoRustBuffer) AsReader() *bytes.Reader {
	b := unsafe.Slice((*byte)(cb.inner.data), C.uint64_t(cb.inner.len))
	return bytes.NewReader(b)
}

func (cb GoRustBuffer) Free() {
	rustCall(func(status *C.RustCallStatus) bool {
		C.ffi_slim_bindings_rustbuffer_free(cb.inner, status)
		return false
	})
}

func (cb GoRustBuffer) ToGoBytes() []byte {
	return C.GoBytes(unsafe.Pointer(cb.inner.data), C.int(cb.inner.len))
}

func stringToRustBuffer(str string) C.RustBuffer {
	return bytesToRustBuffer([]byte(str))
}

func bytesToRustBuffer(b []byte) C.RustBuffer {
	if len(b) == 0 {
		return C.RustBuffer{}
	}
	// We can pass the pointer along here, as it is pinned
	// for the duration of this call
	foreign := C.ForeignBytes{
		len:  C.int(len(b)),
		data: (*C.uchar)(unsafe.Pointer(&b[0])),
	}

	return rustCall(func(status *C.RustCallStatus) C.RustBuffer {
		return C.ffi_slim_bindings_rustbuffer_from_bytes(foreign, status)
	})
}

type BufLifter[GoType any] interface {
	Lift(value RustBufferI) GoType
}

type BufLowerer[GoType any] interface {
	Lower(value GoType) C.RustBuffer
}

type BufReader[GoType any] interface {
	Read(reader io.Reader) GoType
}

type BufWriter[GoType any] interface {
	Write(writer io.Writer, value GoType)
}

func LowerIntoRustBuffer[GoType any](bufWriter BufWriter[GoType], value GoType) C.RustBuffer {
	// This might be not the most efficient way but it does not require knowing allocation size
	// beforehand
	var buffer bytes.Buffer
	bufWriter.Write(&buffer, value)

	bytes, err := io.ReadAll(&buffer)
	if err != nil {
		panic(fmt.Errorf("reading written data: %w", err))
	}
	return bytesToRustBuffer(bytes)
}

func LiftFromRustBuffer[GoType any](bufReader BufReader[GoType], rbuf RustBufferI) GoType {
	defer rbuf.Free()
	reader := rbuf.AsReader()
	item := bufReader.Read(reader)
	if reader.Len() > 0 {
		// TODO: Remove this
		leftover, _ := io.ReadAll(reader)
		panic(fmt.Errorf("Junk remaining in buffer after lifting: %s", string(leftover)))
	}
	return item
}

func rustCallWithError[E any, U any](converter BufReader[*E], callback func(*C.RustCallStatus) U) (U, *E) {
	var status C.RustCallStatus
	returnValue := callback(&status)
	err := checkCallStatus(converter, status)
	return returnValue, err
}

func checkCallStatus[E any](converter BufReader[*E], status C.RustCallStatus) *E {
	switch status.code {
	case 0:
		return nil
	case 1:
		return LiftFromRustBuffer(converter, GoRustBuffer{inner: status.errorBuf})
	case 2:
		// when the rust code sees a panic, it tries to construct a rustBuffer
		// with the message.  but if that code panics, then it just sends back
		// an empty buffer.
		if status.errorBuf.len > 0 {
			panic(fmt.Errorf("%s", FfiConverterStringINSTANCE.Lift(GoRustBuffer{inner: status.errorBuf})))
		} else {
			panic(fmt.Errorf("Rust panicked while handling Rust panic"))
		}
	default:
		panic(fmt.Errorf("unknown status code: %d", status.code))
	}
}

func checkCallStatusUnknown(status C.RustCallStatus) error {
	switch status.code {
	case 0:
		return nil
	case 1:
		panic(fmt.Errorf("function not returning an error returned an error"))
	case 2:
		// when the rust code sees a panic, it tries to construct a C.RustBuffer
		// with the message.  but if that code panics, then it just sends back
		// an empty buffer.
		if status.errorBuf.len > 0 {
			panic(fmt.Errorf("%s", FfiConverterStringINSTANCE.Lift(GoRustBuffer{
				inner: status.errorBuf,
			})))
		} else {
			panic(fmt.Errorf("Rust panicked while handling Rust panic"))
		}
	default:
		return fmt.Errorf("unknown status code: %d", status.code)
	}
}

func rustCall[U any](callback func(*C.RustCallStatus) U) U {
	returnValue, err := rustCallWithError[error](nil, callback)
	if err != nil {
		panic(err)
	}
	return returnValue
}

type NativeError interface {
	AsError() error
}

func writeInt8(writer io.Writer, value int8) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeUint8(writer io.Writer, value uint8) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeInt16(writer io.Writer, value int16) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeUint16(writer io.Writer, value uint16) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeInt32(writer io.Writer, value int32) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeUint32(writer io.Writer, value uint32) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeInt64(writer io.Writer, value int64) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeUint64(writer io.Writer, value uint64) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeFloat32(writer io.Writer, value float32) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func writeFloat64(writer io.Writer, value float64) {
	if err := binary.Write(writer, binary.BigEndian, value); err != nil {
		panic(err)
	}
}

func readInt8(reader io.Reader) int8 {
	var result int8
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readUint8(reader io.Reader) uint8 {
	var result uint8
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readInt16(reader io.Reader) int16 {
	var result int16
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readUint16(reader io.Reader) uint16 {
	var result uint16
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readInt32(reader io.Reader) int32 {
	var result int32
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readUint32(reader io.Reader) uint32 {
	var result uint32
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readInt64(reader io.Reader) int64 {
	var result int64
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readUint64(reader io.Reader) uint64 {
	var result uint64
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readFloat32(reader io.Reader) float32 {
	var result float32
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func readFloat64(reader io.Reader) float64 {
	var result float64
	if err := binary.Read(reader, binary.BigEndian, &result); err != nil {
		panic(err)
	}
	return result
}

func init() {

	uniffiCheckChecksums()
}

func uniffiCheckChecksums() {
	// Get the bindings contract version from our ComponentInterface
	bindingsContractVersion := 26
	// Get the scaffolding contract version by calling the into the dylib
	scaffoldingContractVersion := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint32_t {
		return C.ffi_slim_bindings_uniffi_contract_version()
	})
	if bindingsContractVersion != int(scaffoldingContractVersion) {
		// If this happens try cleaning and rebuilding your project
		panic("slim_bindings: UniFFI contract version mismatch")
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_func_create_app_with_secret()
		})
		if checksum != 11047 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_func_create_app_with_secret: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_func_get_build_info()
		})
		if checksum != 20767 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_func_get_build_info: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_func_get_version()
		})
		if checksum != 28442 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_func_get_version: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_func_initialize_crypto_provider()
		})
		if checksum != 65424 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_func_initialize_crypto_provider: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_func_new_insecure_client_config()
		})
		if checksum != 42525 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_func_new_insecure_client_config: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_func_new_insecure_server_config()
		})
		if checksum != 40258 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_func_new_insecure_server_config: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_func_new_server_config()
		})
		if checksum != 36482 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_func_new_server_config: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_connect()
		})
		if checksum != 27094 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_connect: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_connect_async()
		})
		if checksum != 60988 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_connect_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_create_session()
		})
		if checksum != 27334 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_create_session: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_create_session_and_wait()
		})
		if checksum != 23998 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_create_session_and_wait: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_create_session_and_wait_async()
		})
		if checksum != 10396 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_create_session_and_wait_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_create_session_async()
		})
		if checksum != 11987 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_create_session_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_delete_session()
		})
		if checksum != 46076 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_delete_session: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_delete_session_and_wait()
		})
		if checksum != 30592 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_delete_session_and_wait: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_delete_session_and_wait_async()
		})
		if checksum != 46985 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_delete_session_and_wait_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_delete_session_async()
		})
		if checksum != 34213 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_delete_session_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_disconnect()
		})
		if checksum != 58917 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_disconnect: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_disconnect_async()
		})
		if checksum != 25893 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_disconnect_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_id()
		})
		if checksum != 255 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_id: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_listen_for_session()
		})
		if checksum != 47253 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_listen_for_session: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_listen_for_session_async()
		})
		if checksum != 32970 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_listen_for_session_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_name()
		})
		if checksum != 7693 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_name: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_remove_route()
		})
		if checksum != 37555 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_remove_route: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_remove_route_async()
		})
		if checksum != 4619 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_remove_route_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_run_server()
		})
		if checksum != 10602 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_run_server: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_run_server_async()
		})
		if checksum != 15526 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_run_server_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_set_route()
		})
		if checksum != 17704 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_set_route: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_set_route_async()
		})
		if checksum != 61152 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_set_route_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_stop_server()
		})
		if checksum != 5483 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_stop_server: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_subscribe()
		})
		if checksum != 48651 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_subscribe: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_subscribe_async()
		})
		if checksum != 57830 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_subscribe_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_unsubscribe()
		})
		if checksum != 25780 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_unsubscribe: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_unsubscribe_async()
		})
		if checksum != 40820 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_unsubscribe_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_config()
		})
		if checksum != 23518 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_config: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_destination()
		})
		if checksum != 36250 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_destination: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_get_message()
		})
		if checksum != 32568 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_get_message: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_get_message_async()
		})
		if checksum != 34609 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_get_message_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_invite()
		})
		if checksum != 33714 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_invite: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_invite_and_wait()
		})
		if checksum != 42565 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_invite_and_wait: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_invite_and_wait_async()
		})
		if checksum != 26172 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_invite_and_wait_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_invite_async()
		})
		if checksum != 34826 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_invite_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_is_initiator()
		})
		if checksum != 11659 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_is_initiator: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_metadata()
		})
		if checksum != 33472 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_metadata: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish()
		})
		if checksum != 15851 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_and_wait()
		})
		if checksum != 65167 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_and_wait: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_and_wait_async()
		})
		if checksum != 64246 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_and_wait_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_async()
		})
		if checksum != 17159 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to()
		})
		if checksum != 6249 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to_and_wait()
		})
		if checksum != 47480 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to_and_wait: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to_and_wait_async()
		})
		if checksum != 738 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to_and_wait_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to_async()
		})
		if checksum != 63809 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_with_params()
		})
		if checksum != 47762 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_with_params: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_with_params_async()
		})
		if checksum != 15390 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_with_params_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_remove()
		})
		if checksum != 40003 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_remove: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_remove_and_wait()
		})
		if checksum != 28931 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_remove_and_wait: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_remove_and_wait_async()
		})
		if checksum != 47920 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_remove_and_wait_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_remove_async()
		})
		if checksum != 27248 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_remove_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_session_id()
		})
		if checksum != 33411 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_session_id: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_session_type()
		})
		if checksum != 12628 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_session_type: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_source()
		})
		if checksum != 13106 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_source: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_completionhandle_wait()
		})
		if checksum != 24983 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_completionhandle_wait: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_completionhandle_wait_async()
		})
		if checksum != 35325 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_completionhandle_wait_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_completionhandle_wait_for()
		})
		if checksum != 61981 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_completionhandle_wait_for: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_completionhandle_wait_for_async()
		})
		if checksum != 7758 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_completionhandle_wait_for_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_name_components()
		})
		if checksum != 49977 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_name_components: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_name_id()
		})
		if checksum != 28732 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_name_id: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_constructor_name_new()
		})
		if checksum != 65156 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_constructor_name_new: UniFFI API checksum mismatch")
		}
	}
}

type FfiConverterUint32 struct{}

var FfiConverterUint32INSTANCE = FfiConverterUint32{}

func (FfiConverterUint32) Lower(value uint32) C.uint32_t {
	return C.uint32_t(value)
}

func (FfiConverterUint32) Write(writer io.Writer, value uint32) {
	writeUint32(writer, value)
}

func (FfiConverterUint32) Lift(value C.uint32_t) uint32 {
	return uint32(value)
}

func (FfiConverterUint32) Read(reader io.Reader) uint32 {
	return readUint32(reader)
}

type FfiDestroyerUint32 struct{}

func (FfiDestroyerUint32) Destroy(_ uint32) {}

type FfiConverterUint64 struct{}

var FfiConverterUint64INSTANCE = FfiConverterUint64{}

func (FfiConverterUint64) Lower(value uint64) C.uint64_t {
	return C.uint64_t(value)
}

func (FfiConverterUint64) Write(writer io.Writer, value uint64) {
	writeUint64(writer, value)
}

func (FfiConverterUint64) Lift(value C.uint64_t) uint64 {
	return uint64(value)
}

func (FfiConverterUint64) Read(reader io.Reader) uint64 {
	return readUint64(reader)
}

type FfiDestroyerUint64 struct{}

func (FfiDestroyerUint64) Destroy(_ uint64) {}

type FfiConverterBool struct{}

var FfiConverterBoolINSTANCE = FfiConverterBool{}

func (FfiConverterBool) Lower(value bool) C.int8_t {
	if value {
		return C.int8_t(1)
	}
	return C.int8_t(0)
}

func (FfiConverterBool) Write(writer io.Writer, value bool) {
	if value {
		writeInt8(writer, 1)
	} else {
		writeInt8(writer, 0)
	}
}

func (FfiConverterBool) Lift(value C.int8_t) bool {
	return value != 0
}

func (FfiConverterBool) Read(reader io.Reader) bool {
	return readInt8(reader) != 0
}

type FfiDestroyerBool struct{}

func (FfiDestroyerBool) Destroy(_ bool) {}

type FfiConverterString struct{}

var FfiConverterStringINSTANCE = FfiConverterString{}

func (FfiConverterString) Lift(rb RustBufferI) string {
	defer rb.Free()
	reader := rb.AsReader()
	b, err := io.ReadAll(reader)
	if err != nil {
		panic(fmt.Errorf("reading reader: %w", err))
	}
	return string(b)
}

func (FfiConverterString) Read(reader io.Reader) string {
	length := readInt32(reader)
	buffer := make([]byte, length)
	read_length, err := reader.Read(buffer)
	if err != nil && err != io.EOF {
		panic(err)
	}
	if read_length != int(length) {
		panic(fmt.Errorf("bad read length when reading string, expected %d, read %d", length, read_length))
	}
	return string(buffer)
}

func (FfiConverterString) Lower(value string) C.RustBuffer {
	return stringToRustBuffer(value)
}

func (FfiConverterString) Write(writer io.Writer, value string) {
	if len(value) > math.MaxInt32 {
		panic("String is too large to fit into Int32")
	}

	writeInt32(writer, int32(len(value)))
	write_length, err := io.WriteString(writer, value)
	if err != nil {
		panic(err)
	}
	if write_length != len(value) {
		panic(fmt.Errorf("bad write length when writing string, expected %d, written %d", len(value), write_length))
	}
}

type FfiDestroyerString struct{}

func (FfiDestroyerString) Destroy(_ string) {}

type FfiConverterBytes struct{}

var FfiConverterBytesINSTANCE = FfiConverterBytes{}

func (c FfiConverterBytes) Lower(value []byte) C.RustBuffer {
	return LowerIntoRustBuffer[[]byte](c, value)
}

func (c FfiConverterBytes) Write(writer io.Writer, value []byte) {
	if len(value) > math.MaxInt32 {
		panic("[]byte is too large to fit into Int32")
	}

	writeInt32(writer, int32(len(value)))
	write_length, err := writer.Write(value)
	if err != nil {
		panic(err)
	}
	if write_length != len(value) {
		panic(fmt.Errorf("bad write length when writing []byte, expected %d, written %d", len(value), write_length))
	}
}

func (c FfiConverterBytes) Lift(rb RustBufferI) []byte {
	return LiftFromRustBuffer[[]byte](c, rb)
}

func (c FfiConverterBytes) Read(reader io.Reader) []byte {
	length := readInt32(reader)
	buffer := make([]byte, length)
	read_length, err := reader.Read(buffer)
	if err != nil && err != io.EOF {
		panic(err)
	}
	if read_length != int(length) {
		panic(fmt.Errorf("bad read length when reading []byte, expected %d, read %d", length, read_length))
	}
	return buffer
}

type FfiDestroyerBytes struct{}

func (FfiDestroyerBytes) Destroy(_ []byte) {}

// FfiConverterDuration converts between uniffi duration and Go duration.
type FfiConverterDuration struct{}

var FfiConverterDurationINSTANCE = FfiConverterDuration{}

func (c FfiConverterDuration) Lift(rb RustBufferI) time.Duration {
	return LiftFromRustBuffer[time.Duration](c, rb)
}

func (c FfiConverterDuration) Read(reader io.Reader) time.Duration {
	sec := readUint64(reader)
	nsec := readUint32(reader)
	return time.Duration(sec*1_000_000_000 + uint64(nsec))
}

func (c FfiConverterDuration) Lower(value time.Duration) C.RustBuffer {
	return LowerIntoRustBuffer[time.Duration](c, value)
}

func (c FfiConverterDuration) Write(writer io.Writer, value time.Duration) {
	if value.Nanoseconds() < 0 {
		// Rust does not support negative durations:
		// https://www.reddit.com/r/rust/comments/ljl55u/why_rusts_duration_not_supporting_negative_values/
		// This panic is very bad, because it depends on user input, and in Go user input related
		// error are supposed to be returned as errors, and not cause panics. However, with the
		// current architecture, its not possible to return an error from here, so panic is used as
		// the only other option to signal an error.
		panic("negative duration is not allowed")
	}

	writeUint64(writer, uint64(value)/1_000_000_000)
	writeUint32(writer, uint32(uint64(value)%1_000_000_000))
}

type FfiDestroyerDuration struct{}

func (FfiDestroyerDuration) Destroy(_ time.Duration) {}

// Below is an implementation of synchronization requirements outlined in the link.
// https://github.com/mozilla/uniffi-rs/blob/0dc031132d9493ca812c3af6e7dd60ad2ea95bf0/uniffi_bindgen/src/bindings/kotlin/templates/ObjectRuntime.kt#L31

type FfiObject struct {
	pointer       unsafe.Pointer
	callCounter   atomic.Int64
	cloneFunction func(unsafe.Pointer, *C.RustCallStatus) unsafe.Pointer
	freeFunction  func(unsafe.Pointer, *C.RustCallStatus)
	destroyed     atomic.Bool
}

func newFfiObject(
	pointer unsafe.Pointer,
	cloneFunction func(unsafe.Pointer, *C.RustCallStatus) unsafe.Pointer,
	freeFunction func(unsafe.Pointer, *C.RustCallStatus),
) FfiObject {
	return FfiObject{
		pointer:       pointer,
		cloneFunction: cloneFunction,
		freeFunction:  freeFunction,
	}
}

func (ffiObject *FfiObject) incrementPointer(debugName string) unsafe.Pointer {
	for {
		counter := ffiObject.callCounter.Load()
		if counter <= -1 {
			panic(fmt.Errorf("%v object has already been destroyed", debugName))
		}
		if counter == math.MaxInt64 {
			panic(fmt.Errorf("%v object call counter would overflow", debugName))
		}
		if ffiObject.callCounter.CompareAndSwap(counter, counter+1) {
			break
		}
	}

	return rustCall(func(status *C.RustCallStatus) unsafe.Pointer {
		return ffiObject.cloneFunction(ffiObject.pointer, status)
	})
}

func (ffiObject *FfiObject) decrementPointer() {
	if ffiObject.callCounter.Add(-1) == -1 {
		ffiObject.freeRustArcPtr()
	}
}

func (ffiObject *FfiObject) destroy() {
	if ffiObject.destroyed.CompareAndSwap(false, true) {
		if ffiObject.callCounter.Add(-1) == -1 {
			ffiObject.freeRustArcPtr()
		}
	}
}

func (ffiObject *FfiObject) freeRustArcPtr() {
	rustCall(func(status *C.RustCallStatus) int32 {
		ffiObject.freeFunction(ffiObject.pointer, status)
		return 0
	})
}

// Adapter that bridges the App API with language-bindings interface
//
// This adapter uses enum-based auth types (`AuthProvider`/`AuthVerifier`) instead of generics
// to be compatible with UniFFI, supporting multiple authentication mechanisms (SharedSecret,
// JWT, SPIRE, StaticToken). It provides both synchronous (blocking) and asynchronous methods
// for flexibility.
type BindingsAdapterInterface interface {
	// Connect to a SLIM server as a client (blocking version for FFI)
	//
	// # Arguments
	// * `config` - Client configuration (endpoint and TLS settings)
	//
	// # Returns
	// * `Ok(connection_id)` - Connected successfully, returns the connection ID
	// * `Err(SlimError)` - If connection fails
	Connect(config ClientConfig) (uint64, error)
	// Connect to a SLIM server (async version)
	//
	// Note: Automatically subscribes to the app's own name after connecting
	// to enable receiving inbound messages and sessions.
	ConnectAsync(config ClientConfig) (uint64, error)
	// Create a new session (blocking version for FFI)
	//
	// Returns a SessionWithCompletion containing the session context and a completion handle.
	// Call `.wait()` on the completion handle to wait for session establishment.
	CreateSession(config SessionConfig, destination *Name) (SessionWithCompletion, error)
	// Create a new session and wait for completion (blocking version)
	//
	// This method creates a session and blocks until the session establishment completes.
	// Returns only the session context, as the completion has already been awaited.
	CreateSessionAndWait(config SessionConfig, destination *Name) (*BindingsSessionContext, error)
	// Create a new session and wait for completion (async version)
	//
	// This method creates a session and waits until the session establishment completes.
	// Returns only the session context, as the completion has already been awaited.
	CreateSessionAndWaitAsync(config SessionConfig, destination *Name) (*BindingsSessionContext, error)
	// Create a new session (async version)
	//
	// Returns a SessionWithCompletion containing the session context and a completion handle.
	// Await the completion handle to wait for session establishment.
	// For point-to-point sessions, this ensures the remote peer has acknowledged the session.
	// For multicast sessions, this ensures the initial setup is complete.
	CreateSessionAsync(config SessionConfig, destination *Name) (SessionWithCompletion, error)
	// Delete a session (blocking version for FFI)
	//
	// Returns a completion handle that can be awaited to ensure the deletion completes.
	DeleteSession(session *BindingsSessionContext) (*CompletionHandle, error)
	// Delete a session and wait for completion (blocking version)
	//
	// This method deletes a session and blocks until the deletion completes.
	DeleteSessionAndWait(session *BindingsSessionContext) error
	// Delete a session and wait for completion (async version)
	//
	// This method deletes a session and waits until the deletion completes.
	DeleteSessionAndWaitAsync(session *BindingsSessionContext) error
	// Delete a session (async version)
	//
	// Returns a completion handle that can be awaited to ensure the deletion completes.
	DeleteSessionAsync(session *BindingsSessionContext) (*CompletionHandle, error)
	// Disconnect from a SLIM server (blocking version for FFI)
	//
	// # Arguments
	// * `connection_id` - The connection ID returned from `connect()`
	//
	// # Returns
	// * `Ok(())` - Disconnected successfully
	// * `Err(SlimError)` - If disconnection fails
	Disconnect(connectionId uint64) error
	// Disconnect from a SLIM server (async version)
	DisconnectAsync(connectionId uint64) error
	// Get the app ID (derived from name)
	Id() uint64
	// Listen for incoming sessions (blocking version for FFI)
	ListenForSession(timeout *time.Duration) (*BindingsSessionContext, error)
	// Listen for incoming sessions (async version)
	ListenForSessionAsync(timeout *time.Duration) (*BindingsSessionContext, error)
	// Get the app name
	Name() *Name
	// Remove a route (blocking version for FFI)
	RemoveRoute(name *Name, connectionId uint64) error
	// Remove a route (async version)
	RemoveRouteAsync(name *Name, connectionId uint64) error
	// Run a SLIM server on the specified endpoint (blocking version for FFI)
	//
	// # Arguments
	// * `config` - Server configuration (endpoint and TLS settings)
	//
	// # Returns
	// * `Ok(())` - Server started successfully
	// * `Err(SlimError)` - If server startup fails
	RunServer(config ServerConfig) error
	// Run a SLIM server (async version)
	RunServerAsync(config ServerConfig) error
	// Set a route to a name for a specific connection (blocking version for FFI)
	SetRoute(name *Name, connectionId uint64) error
	// Set a route to a name for a specific connection (async version)
	SetRouteAsync(name *Name, connectionId uint64) error
	// Stop a running SLIM server (blocking version for FFI)
	//
	// # Arguments
	// * `endpoint` - The endpoint address of the server to stop (e.g., "127.0.0.1:12345")
	//
	// # Returns
	// * `Ok(())` - Server stopped successfully
	// * `Err(SlimError)` - If server not found or stop fails
	StopServer(endpoint string) error
	// Subscribe to a session name (blocking version for FFI)
	Subscribe(name *Name, connectionId *uint64) error
	// Subscribe to a name (async version)
	SubscribeAsync(name *Name, connectionId *uint64) error
	// Unsubscribe from a name (blocking version for FFI)
	Unsubscribe(name *Name, connectionId *uint64) error
	// Unsubscribe from a name (async version)
	UnsubscribeAsync(name *Name, connectionId *uint64) error
}

// Adapter that bridges the App API with language-bindings interface
//
// This adapter uses enum-based auth types (`AuthProvider`/`AuthVerifier`) instead of generics
// to be compatible with UniFFI, supporting multiple authentication mechanisms (SharedSecret,
// JWT, SPIRE, StaticToken). It provides both synchronous (blocking) and asynchronous methods
// for flexibility.
type BindingsAdapter struct {
	ffiObject FfiObject
}

// Connect to a SLIM server as a client (blocking version for FFI)
//
// # Arguments
// * `config` - Client configuration (endpoint and TLS settings)
//
// # Returns
// * `Ok(connection_id)` - Connected successfully, returns the connection ID
// * `Err(SlimError)` - If connection fails
func (_self *BindingsAdapter) Connect(config ClientConfig) (uint64, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_slim_bindings_fn_method_bindingsadapter_connect(
			_pointer, FfiConverterClientConfigINSTANCE.Lower(config), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue uint64
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterUint64INSTANCE.Lift(_uniffiRV), nil
	}
}

// Connect to a SLIM server (async version)
//
// Note: Automatically subscribes to the app's own name after connecting
// to enable receiving inbound messages and sessions.
func (_self *BindingsAdapter) ConnectAsync(config ClientConfig) (uint64, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) C.uint64_t {
			res := C.ffi_slim_bindings_rust_future_complete_u64(handle, status)
			return res
		},
		// liftFn
		func(ffi C.uint64_t) uint64 {
			return FfiConverterUint64INSTANCE.Lift(ffi)
		},
		C.uniffi_slim_bindings_fn_method_bindingsadapter_connect_async(
			_pointer, FfiConverterClientConfigINSTANCE.Lower(config)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_u64(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_u64(handle)
		},
	)

	return res, err
}

// Create a new session (blocking version for FFI)
//
// Returns a SessionWithCompletion containing the session context and a completion handle.
// Call `.wait()` on the completion handle to wait for session establishment.
func (_self *BindingsAdapter) CreateSession(config SessionConfig, destination *Name) (SessionWithCompletion, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_bindings_fn_method_bindingsadapter_create_session(
				_pointer, FfiConverterSessionConfigINSTANCE.Lower(config), FfiConverterNameINSTANCE.Lower(destination), _uniffiStatus),
		}
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue SessionWithCompletion
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterSessionWithCompletionINSTANCE.Lift(_uniffiRV), nil
	}
}

// Create a new session and wait for completion (blocking version)
//
// This method creates a session and blocks until the session establishment completes.
// Returns only the session context, as the completion has already been awaited.
func (_self *BindingsAdapter) CreateSessionAndWait(config SessionConfig, destination *Name) (*BindingsSessionContext, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_method_bindingsadapter_create_session_and_wait(
			_pointer, FfiConverterSessionConfigINSTANCE.Lower(config), FfiConverterNameINSTANCE.Lower(destination), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *BindingsSessionContext
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterBindingsSessionContextINSTANCE.Lift(_uniffiRV), nil
	}
}

// Create a new session and wait for completion (async version)
//
// This method creates a session and waits until the session establishment completes.
// Returns only the session context, as the completion has already been awaited.
func (_self *BindingsAdapter) CreateSessionAndWaitAsync(config SessionConfig, destination *Name) (*BindingsSessionContext, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) unsafe.Pointer {
			res := C.ffi_slim_bindings_rust_future_complete_pointer(handle, status)
			return res
		},
		// liftFn
		func(ffi unsafe.Pointer) *BindingsSessionContext {
			return FfiConverterBindingsSessionContextINSTANCE.Lift(ffi)
		},
		C.uniffi_slim_bindings_fn_method_bindingsadapter_create_session_and_wait_async(
			_pointer, FfiConverterSessionConfigINSTANCE.Lower(config), FfiConverterNameINSTANCE.Lower(destination)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_pointer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_pointer(handle)
		},
	)

	return res, err
}

// Create a new session (async version)
//
// Returns a SessionWithCompletion containing the session context and a completion handle.
// Await the completion handle to wait for session establishment.
// For point-to-point sessions, this ensures the remote peer has acknowledged the session.
// For multicast sessions, this ensures the initial setup is complete.
func (_self *BindingsAdapter) CreateSessionAsync(config SessionConfig, destination *Name) (SessionWithCompletion, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) RustBufferI {
			res := C.ffi_slim_bindings_rust_future_complete_rust_buffer(handle, status)
			return GoRustBuffer{
				inner: res,
			}
		},
		// liftFn
		func(ffi RustBufferI) SessionWithCompletion {
			return FfiConverterSessionWithCompletionINSTANCE.Lift(ffi)
		},
		C.uniffi_slim_bindings_fn_method_bindingsadapter_create_session_async(
			_pointer, FfiConverterSessionConfigINSTANCE.Lower(config), FfiConverterNameINSTANCE.Lower(destination)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_rust_buffer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_rust_buffer(handle)
		},
	)

	return res, err
}

// Delete a session (blocking version for FFI)
//
// Returns a completion handle that can be awaited to ensure the deletion completes.
func (_self *BindingsAdapter) DeleteSession(session *BindingsSessionContext) (*CompletionHandle, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_method_bindingsadapter_delete_session(
			_pointer, FfiConverterBindingsSessionContextINSTANCE.Lower(session), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *CompletionHandle
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterCompletionHandleINSTANCE.Lift(_uniffiRV), nil
	}
}

// Delete a session and wait for completion (blocking version)
//
// This method deletes a session and blocks until the deletion completes.
func (_self *BindingsAdapter) DeleteSessionAndWait(session *BindingsSessionContext) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingsadapter_delete_session_and_wait(
			_pointer, FfiConverterBindingsSessionContextINSTANCE.Lower(session), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Delete a session and wait for completion (async version)
//
// This method deletes a session and waits until the deletion completes.
func (_self *BindingsAdapter) DeleteSessionAndWaitAsync(session *BindingsSessionContext) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_bindings_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_bindings_fn_method_bindingsadapter_delete_session_and_wait_async(
			_pointer, FfiConverterBindingsSessionContextINSTANCE.Lower(session)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_void(handle)
		},
	)

	return err
}

// Delete a session (async version)
//
// Returns a completion handle that can be awaited to ensure the deletion completes.
func (_self *BindingsAdapter) DeleteSessionAsync(session *BindingsSessionContext) (*CompletionHandle, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) unsafe.Pointer {
			res := C.ffi_slim_bindings_rust_future_complete_pointer(handle, status)
			return res
		},
		// liftFn
		func(ffi unsafe.Pointer) *CompletionHandle {
			return FfiConverterCompletionHandleINSTANCE.Lift(ffi)
		},
		C.uniffi_slim_bindings_fn_method_bindingsadapter_delete_session_async(
			_pointer, FfiConverterBindingsSessionContextINSTANCE.Lower(session)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_pointer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_pointer(handle)
		},
	)

	return res, err
}

// Disconnect from a SLIM server (blocking version for FFI)
//
// # Arguments
// * `connection_id` - The connection ID returned from `connect()`
//
// # Returns
// * `Ok(())` - Disconnected successfully
// * `Err(SlimError)` - If disconnection fails
func (_self *BindingsAdapter) Disconnect(connectionId uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingsadapter_disconnect(
			_pointer, FfiConverterUint64INSTANCE.Lower(connectionId), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Disconnect from a SLIM server (async version)
func (_self *BindingsAdapter) DisconnectAsync(connectionId uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_bindings_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_bindings_fn_method_bindingsadapter_disconnect_async(
			_pointer, FfiConverterUint64INSTANCE.Lower(connectionId)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_void(handle)
		},
	)

	return err
}

// Get the app ID (derived from name)
func (_self *BindingsAdapter) Id() uint64 {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterUint64INSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_slim_bindings_fn_method_bindingsadapter_id(
			_pointer, _uniffiStatus)
	}))
}

// Listen for incoming sessions (blocking version for FFI)
func (_self *BindingsAdapter) ListenForSession(timeout *time.Duration) (*BindingsSessionContext, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_method_bindingsadapter_listen_for_session(
			_pointer, FfiConverterOptionalDurationINSTANCE.Lower(timeout), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *BindingsSessionContext
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterBindingsSessionContextINSTANCE.Lift(_uniffiRV), nil
	}
}

// Listen for incoming sessions (async version)
func (_self *BindingsAdapter) ListenForSessionAsync(timeout *time.Duration) (*BindingsSessionContext, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) unsafe.Pointer {
			res := C.ffi_slim_bindings_rust_future_complete_pointer(handle, status)
			return res
		},
		// liftFn
		func(ffi unsafe.Pointer) *BindingsSessionContext {
			return FfiConverterBindingsSessionContextINSTANCE.Lift(ffi)
		},
		C.uniffi_slim_bindings_fn_method_bindingsadapter_listen_for_session_async(
			_pointer, FfiConverterOptionalDurationINSTANCE.Lower(timeout)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_pointer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_pointer(handle)
		},
	)

	return res, err
}

// Get the app name
func (_self *BindingsAdapter) Name() *Name {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterNameINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_method_bindingsadapter_name(
			_pointer, _uniffiStatus)
	}))
}

// Remove a route (blocking version for FFI)
func (_self *BindingsAdapter) RemoveRoute(name *Name, connectionId uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingsadapter_remove_route(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterUint64INSTANCE.Lower(connectionId), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Remove a route (async version)
func (_self *BindingsAdapter) RemoveRouteAsync(name *Name, connectionId uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_bindings_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_bindings_fn_method_bindingsadapter_remove_route_async(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterUint64INSTANCE.Lower(connectionId)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_void(handle)
		},
	)

	return err
}

// Run a SLIM server on the specified endpoint (blocking version for FFI)
//
// # Arguments
// * `config` - Server configuration (endpoint and TLS settings)
//
// # Returns
// * `Ok(())` - Server started successfully
// * `Err(SlimError)` - If server startup fails
func (_self *BindingsAdapter) RunServer(config ServerConfig) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingsadapter_run_server(
			_pointer, FfiConverterServerConfigINSTANCE.Lower(config), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Run a SLIM server (async version)
func (_self *BindingsAdapter) RunServerAsync(config ServerConfig) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_bindings_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_bindings_fn_method_bindingsadapter_run_server_async(
			_pointer, FfiConverterServerConfigINSTANCE.Lower(config)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_void(handle)
		},
	)

	return err
}

// Set a route to a name for a specific connection (blocking version for FFI)
func (_self *BindingsAdapter) SetRoute(name *Name, connectionId uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingsadapter_set_route(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterUint64INSTANCE.Lower(connectionId), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Set a route to a name for a specific connection (async version)
func (_self *BindingsAdapter) SetRouteAsync(name *Name, connectionId uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_bindings_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_bindings_fn_method_bindingsadapter_set_route_async(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterUint64INSTANCE.Lower(connectionId)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_void(handle)
		},
	)

	return err
}

// Stop a running SLIM server (blocking version for FFI)
//
// # Arguments
// * `endpoint` - The endpoint address of the server to stop (e.g., "127.0.0.1:12345")
//
// # Returns
// * `Ok(())` - Server stopped successfully
// * `Err(SlimError)` - If server not found or stop fails
func (_self *BindingsAdapter) StopServer(endpoint string) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingsadapter_stop_server(
			_pointer, FfiConverterStringINSTANCE.Lower(endpoint), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Subscribe to a session name (blocking version for FFI)
func (_self *BindingsAdapter) Subscribe(name *Name, connectionId *uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingsadapter_subscribe(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterOptionalUint64INSTANCE.Lower(connectionId), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Subscribe to a name (async version)
func (_self *BindingsAdapter) SubscribeAsync(name *Name, connectionId *uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_bindings_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_bindings_fn_method_bindingsadapter_subscribe_async(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterOptionalUint64INSTANCE.Lower(connectionId)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_void(handle)
		},
	)

	return err
}

// Unsubscribe from a name (blocking version for FFI)
func (_self *BindingsAdapter) Unsubscribe(name *Name, connectionId *uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingsadapter_unsubscribe(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterOptionalUint64INSTANCE.Lower(connectionId), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Unsubscribe from a name (async version)
func (_self *BindingsAdapter) UnsubscribeAsync(name *Name, connectionId *uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_bindings_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_bindings_fn_method_bindingsadapter_unsubscribe_async(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterOptionalUint64INSTANCE.Lower(connectionId)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_void(handle)
		},
	)

	return err
}
func (object *BindingsAdapter) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterBindingsAdapter struct{}

var FfiConverterBindingsAdapterINSTANCE = FfiConverterBindingsAdapter{}

func (c FfiConverterBindingsAdapter) Lift(pointer unsafe.Pointer) *BindingsAdapter {
	result := &BindingsAdapter{
		newFfiObject(
			pointer,
			func(pointer unsafe.Pointer, status *C.RustCallStatus) unsafe.Pointer {
				return C.uniffi_slim_bindings_fn_clone_bindingsadapter(pointer, status)
			},
			func(pointer unsafe.Pointer, status *C.RustCallStatus) {
				C.uniffi_slim_bindings_fn_free_bindingsadapter(pointer, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*BindingsAdapter).Destroy)
	return result
}

func (c FfiConverterBindingsAdapter) Read(reader io.Reader) *BindingsAdapter {
	return c.Lift(unsafe.Pointer(uintptr(readUint64(reader))))
}

func (c FfiConverterBindingsAdapter) Lower(value *BindingsAdapter) unsafe.Pointer {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the pointer will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked pointer.
	pointer := value.ffiObject.incrementPointer("*BindingsAdapter")
	defer value.ffiObject.decrementPointer()
	return pointer

}

func (c FfiConverterBindingsAdapter) Write(writer io.Writer, value *BindingsAdapter) {
	writeUint64(writer, uint64(uintptr(c.Lower(value))))
}

type FfiDestroyerBindingsAdapter struct{}

func (_ FfiDestroyerBindingsAdapter) Destroy(value *BindingsAdapter) {
	value.Destroy()
}

// Session context for language bindings (UniFFI-compatible)
//
// Wraps the session context with proper async access patterns for message reception.
// Provides both synchronous (blocking) and asynchronous methods for FFI compatibility.
type BindingsSessionContextInterface interface {
	// Get the session configuration
	Config() (SessionConfig, error)
	// Get the destination name for this session
	Destination() (*Name, error)
	// Receive a message from the session (blocking version for FFI)
	//
	// # Arguments
	// * `timeout_ms` - Optional timeout in milliseconds
	//
	// # Returns
	// * `Ok(ReceivedMessage)` - Message with context and payload bytes
	// * `Err(SlimError)` - If the receive fails or times out
	GetMessage(timeoutMs *uint32) (ReceivedMessage, error)
	// Receive a message from the session (async version)
	GetMessageAsync(timeoutMs *uint32) (ReceivedMessage, error)
	// Invite a participant to the session (blocking version)
	//
	// Returns a completion handle that can be awaited to ensure the invitation completes.
	Invite(participant *Name) (*CompletionHandle, error)
	// Invite a participant and wait for completion (blocking version)
	//
	// This method invites a participant and blocks until the invitation completes.
	InviteAndWait(participant *Name) error
	// Invite a participant and wait for completion (async version)
	//
	// This method invites a participant and waits until the invitation completes.
	InviteAndWaitAsync(participant *Name) error
	// Invite a participant to the session (async version)
	//
	// Returns a completion handle that can be awaited to ensure the invitation completes.
	InviteAsync(participant *Name) (*CompletionHandle, error)
	// Check if this session is the initiator
	IsInitiator() (bool, error)
	// Get the session metadata
	Metadata() (map[string]string, error)
	// Publish a message to the session's destination (blocking version)
	//
	// Returns a completion handle that can be awaited to ensure the message was delivered.
	//
	// # Arguments
	// * `data` - The message payload bytes
	// * `payload_type` - Optional content type identifier
	// * `metadata` - Optional key-value metadata pairs
	//
	// # Returns
	// * `Ok(CompletionHandle)` - Handle to await delivery confirmation
	// * `Err(SlimError)` - If publishing fails
	//
	// # Example
	// ```ignore
	// let completion = session.publish(data, None, None)?;
	// completion.wait()?; // Blocks until message is delivered
	// ```
	Publish(data []byte, payloadType *string, metadata *map[string]string) (*CompletionHandle, error)
	// Publish a message and wait for completion (blocking version)
	//
	// This method publishes a message and blocks until the delivery completes.
	PublishAndWait(data []byte, payloadType *string, metadata *map[string]string) error
	// Publish a message and wait for completion (async version)
	//
	// This method publishes a message and waits until the delivery completes.
	PublishAndWaitAsync(data []byte, payloadType *string, metadata *map[string]string) error
	// Publish a message to the session's destination (async version)
	//
	// Returns a completion handle that can be awaited to ensure the message was delivered.
	PublishAsync(data []byte, payloadType *string, metadata *map[string]string) (*CompletionHandle, error)
	// Publish a reply message to the originator of a received message (blocking version for FFI)
	//
	// This method uses the routing information from a previously received message
	// to send a reply back to the sender. This is the preferred way to implement
	// request/reply patterns.
	//
	// Returns a completion handle that can be awaited to ensure the message was delivered.
	//
	// # Arguments
	// * `message_context` - Context from a message received via `get_message()`
	// * `data` - The reply payload bytes
	// * `payload_type` - Optional content type identifier
	// * `metadata` - Optional key-value metadata pairs
	//
	// # Returns
	// * `Ok(CompletionHandle)` - Handle to await delivery confirmation
	// * `Err(SlimError)` - If publishing fails
	PublishTo(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) (*CompletionHandle, error)
	// Publish a reply message and wait for completion (blocking version)
	//
	// This method publishes a reply to a received message and blocks until the delivery completes.
	PublishToAndWait(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) error
	// Publish a reply message and wait for completion (async version)
	//
	// This method publishes a reply to a received message and waits until the delivery completes.
	PublishToAndWaitAsync(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) error
	// Publish a reply message (async version)
	//
	// Returns a completion handle that can be awaited to ensure the message was delivered.
	PublishToAsync(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) (*CompletionHandle, error)
	// Low-level publish with full control over all parameters (blocking version for FFI)
	//
	// This is an advanced method that provides complete control over routing and delivery.
	// Most users should use `publish()` or `publish_to()` instead.
	//
	// # Arguments
	// * `destination` - Target name to send to
	// * `fanout` - Number of copies to send (for multicast)
	// * `data` - The message payload bytes
	// * `connection_out` - Optional specific connection ID to use
	// * `payload_type` - Optional content type identifier
	// * `metadata` - Optional key-value metadata pairs
	PublishWithParams(destination *Name, fanout uint32, data []byte, connectionOut *uint64, payloadType *string, metadata *map[string]string) error
	// Low-level publish with full control (async version)
	PublishWithParamsAsync(destination *Name, fanout uint32, data []byte, connectionOut *uint64, payloadType *string, metadata *map[string]string) error
	// Remove a participant from the session (blocking version)
	//
	// Returns a completion handle that can be awaited to ensure the removal completes.
	Remove(participant *Name) (*CompletionHandle, error)
	// Remove a participant and wait for completion (blocking version)
	//
	// This method removes a participant and blocks until the removal completes.
	RemoveAndWait(participant *Name) error
	// Remove a participant and wait for completion (async version)
	//
	// This method removes a participant and waits until the removal completes.
	RemoveAndWaitAsync(participant *Name) error
	// Remove a participant from the session (async version)
	//
	// Returns a completion handle that can be awaited to ensure the removal completes.
	RemoveAsync(participant *Name) (*CompletionHandle, error)
	// Get the session ID
	SessionId() (uint32, error)
	// Get the session type (PointToPoint or Group)
	SessionType() (SessionType, error)
	// Get the source name for this session
	Source() (*Name, error)
}

// Session context for language bindings (UniFFI-compatible)
//
// Wraps the session context with proper async access patterns for message reception.
// Provides both synchronous (blocking) and asynchronous methods for FFI compatibility.
type BindingsSessionContext struct {
	ffiObject FfiObject
}

// Get the session configuration
func (_self *BindingsSessionContext) Config() (SessionConfig, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_bindings_fn_method_bindingssessioncontext_config(
				_pointer, _uniffiStatus),
		}
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue SessionConfig
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterSessionConfigINSTANCE.Lift(_uniffiRV), nil
	}
}

// Get the destination name for this session
func (_self *BindingsSessionContext) Destination() (*Name, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_method_bindingssessioncontext_destination(
			_pointer, _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *Name
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterNameINSTANCE.Lift(_uniffiRV), nil
	}
}

// Receive a message from the session (blocking version for FFI)
//
// # Arguments
// * `timeout_ms` - Optional timeout in milliseconds
//
// # Returns
// * `Ok(ReceivedMessage)` - Message with context and payload bytes
// * `Err(SlimError)` - If the receive fails or times out
func (_self *BindingsSessionContext) GetMessage(timeoutMs *uint32) (ReceivedMessage, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_bindings_fn_method_bindingssessioncontext_get_message(
				_pointer, FfiConverterOptionalUint32INSTANCE.Lower(timeoutMs), _uniffiStatus),
		}
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue ReceivedMessage
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterReceivedMessageINSTANCE.Lift(_uniffiRV), nil
	}
}

// Receive a message from the session (async version)
func (_self *BindingsSessionContext) GetMessageAsync(timeoutMs *uint32) (ReceivedMessage, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) RustBufferI {
			res := C.ffi_slim_bindings_rust_future_complete_rust_buffer(handle, status)
			return GoRustBuffer{
				inner: res,
			}
		},
		// liftFn
		func(ffi RustBufferI) ReceivedMessage {
			return FfiConverterReceivedMessageINSTANCE.Lift(ffi)
		},
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_get_message_async(
			_pointer, FfiConverterOptionalUint32INSTANCE.Lower(timeoutMs)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_rust_buffer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_rust_buffer(handle)
		},
	)

	return res, err
}

// Invite a participant to the session (blocking version)
//
// Returns a completion handle that can be awaited to ensure the invitation completes.
func (_self *BindingsSessionContext) Invite(participant *Name) (*CompletionHandle, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_method_bindingssessioncontext_invite(
			_pointer, FfiConverterNameINSTANCE.Lower(participant), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *CompletionHandle
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterCompletionHandleINSTANCE.Lift(_uniffiRV), nil
	}
}

// Invite a participant and wait for completion (blocking version)
//
// This method invites a participant and blocks until the invitation completes.
func (_self *BindingsSessionContext) InviteAndWait(participant *Name) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_invite_and_wait(
			_pointer, FfiConverterNameINSTANCE.Lower(participant), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Invite a participant and wait for completion (async version)
//
// This method invites a participant and waits until the invitation completes.
func (_self *BindingsSessionContext) InviteAndWaitAsync(participant *Name) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_bindings_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_invite_and_wait_async(
			_pointer, FfiConverterNameINSTANCE.Lower(participant)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_void(handle)
		},
	)

	return err
}

// Invite a participant to the session (async version)
//
// Returns a completion handle that can be awaited to ensure the invitation completes.
func (_self *BindingsSessionContext) InviteAsync(participant *Name) (*CompletionHandle, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) unsafe.Pointer {
			res := C.ffi_slim_bindings_rust_future_complete_pointer(handle, status)
			return res
		},
		// liftFn
		func(ffi unsafe.Pointer) *CompletionHandle {
			return FfiConverterCompletionHandleINSTANCE.Lift(ffi)
		},
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_invite_async(
			_pointer, FfiConverterNameINSTANCE.Lower(participant)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_pointer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_pointer(handle)
		},
	)

	return res, err
}

// Check if this session is the initiator
func (_self *BindingsSessionContext) IsInitiator() (bool, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) C.int8_t {
		return C.uniffi_slim_bindings_fn_method_bindingssessioncontext_is_initiator(
			_pointer, _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue bool
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterBoolINSTANCE.Lift(_uniffiRV), nil
	}
}

// Get the session metadata
func (_self *BindingsSessionContext) Metadata() (map[string]string, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_bindings_fn_method_bindingssessioncontext_metadata(
				_pointer, _uniffiStatus),
		}
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue map[string]string
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterMapStringStringINSTANCE.Lift(_uniffiRV), nil
	}
}

// Publish a message to the session's destination (blocking version)
//
// Returns a completion handle that can be awaited to ensure the message was delivered.
//
// # Arguments
// * `data` - The message payload bytes
// * `payload_type` - Optional content type identifier
// * `metadata` - Optional key-value metadata pairs
//
// # Returns
// * `Ok(CompletionHandle)` - Handle to await delivery confirmation
// * `Err(SlimError)` - If publishing fails
//
// # Example
// ```ignore
// let completion = session.publish(data, None, None)?;
// completion.wait()?; // Blocks until message is delivered
// ```
func (_self *BindingsSessionContext) Publish(data []byte, payloadType *string, metadata *map[string]string) (*CompletionHandle, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish(
			_pointer, FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *CompletionHandle
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterCompletionHandleINSTANCE.Lift(_uniffiRV), nil
	}
}

// Publish a message and wait for completion (blocking version)
//
// This method publishes a message and blocks until the delivery completes.
func (_self *BindingsSessionContext) PublishAndWait(data []byte, payloadType *string, metadata *map[string]string) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_and_wait(
			_pointer, FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Publish a message and wait for completion (async version)
//
// This method publishes a message and waits until the delivery completes.
func (_self *BindingsSessionContext) PublishAndWaitAsync(data []byte, payloadType *string, metadata *map[string]string) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_bindings_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_and_wait_async(
			_pointer, FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_void(handle)
		},
	)

	return err
}

// Publish a message to the session's destination (async version)
//
// Returns a completion handle that can be awaited to ensure the message was delivered.
func (_self *BindingsSessionContext) PublishAsync(data []byte, payloadType *string, metadata *map[string]string) (*CompletionHandle, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) unsafe.Pointer {
			res := C.ffi_slim_bindings_rust_future_complete_pointer(handle, status)
			return res
		},
		// liftFn
		func(ffi unsafe.Pointer) *CompletionHandle {
			return FfiConverterCompletionHandleINSTANCE.Lift(ffi)
		},
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_async(
			_pointer, FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_pointer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_pointer(handle)
		},
	)

	return res, err
}

// Publish a reply message to the originator of a received message (blocking version for FFI)
//
// This method uses the routing information from a previously received message
// to send a reply back to the sender. This is the preferred way to implement
// request/reply patterns.
//
// Returns a completion handle that can be awaited to ensure the message was delivered.
//
// # Arguments
// * `message_context` - Context from a message received via `get_message()`
// * `data` - The reply payload bytes
// * `payload_type` - Optional content type identifier
// * `metadata` - Optional key-value metadata pairs
//
// # Returns
// * `Ok(CompletionHandle)` - Handle to await delivery confirmation
// * `Err(SlimError)` - If publishing fails
func (_self *BindingsSessionContext) PublishTo(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) (*CompletionHandle, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_to(
			_pointer, FfiConverterMessageContextINSTANCE.Lower(messageContext), FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *CompletionHandle
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterCompletionHandleINSTANCE.Lift(_uniffiRV), nil
	}
}

// Publish a reply message and wait for completion (blocking version)
//
// This method publishes a reply to a received message and blocks until the delivery completes.
func (_self *BindingsSessionContext) PublishToAndWait(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_to_and_wait(
			_pointer, FfiConverterMessageContextINSTANCE.Lower(messageContext), FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Publish a reply message and wait for completion (async version)
//
// This method publishes a reply to a received message and waits until the delivery completes.
func (_self *BindingsSessionContext) PublishToAndWaitAsync(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_bindings_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_to_and_wait_async(
			_pointer, FfiConverterMessageContextINSTANCE.Lower(messageContext), FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_void(handle)
		},
	)

	return err
}

// Publish a reply message (async version)
//
// Returns a completion handle that can be awaited to ensure the message was delivered.
func (_self *BindingsSessionContext) PublishToAsync(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) (*CompletionHandle, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) unsafe.Pointer {
			res := C.ffi_slim_bindings_rust_future_complete_pointer(handle, status)
			return res
		},
		// liftFn
		func(ffi unsafe.Pointer) *CompletionHandle {
			return FfiConverterCompletionHandleINSTANCE.Lift(ffi)
		},
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_to_async(
			_pointer, FfiConverterMessageContextINSTANCE.Lower(messageContext), FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_pointer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_pointer(handle)
		},
	)

	return res, err
}

// Low-level publish with full control over all parameters (blocking version for FFI)
//
// This is an advanced method that provides complete control over routing and delivery.
// Most users should use `publish()` or `publish_to()` instead.
//
// # Arguments
// * `destination` - Target name to send to
// * `fanout` - Number of copies to send (for multicast)
// * `data` - The message payload bytes
// * `connection_out` - Optional specific connection ID to use
// * `payload_type` - Optional content type identifier
// * `metadata` - Optional key-value metadata pairs
func (_self *BindingsSessionContext) PublishWithParams(destination *Name, fanout uint32, data []byte, connectionOut *uint64, payloadType *string, metadata *map[string]string) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_with_params(
			_pointer, FfiConverterNameINSTANCE.Lower(destination), FfiConverterUint32INSTANCE.Lower(fanout), FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalUint64INSTANCE.Lower(connectionOut), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Low-level publish with full control (async version)
func (_self *BindingsSessionContext) PublishWithParamsAsync(destination *Name, fanout uint32, data []byte, connectionOut *uint64, payloadType *string, metadata *map[string]string) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_bindings_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_with_params_async(
			_pointer, FfiConverterNameINSTANCE.Lower(destination), FfiConverterUint32INSTANCE.Lower(fanout), FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalUint64INSTANCE.Lower(connectionOut), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_void(handle)
		},
	)

	return err
}

// Remove a participant from the session (blocking version)
//
// Returns a completion handle that can be awaited to ensure the removal completes.
func (_self *BindingsSessionContext) Remove(participant *Name) (*CompletionHandle, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_method_bindingssessioncontext_remove(
			_pointer, FfiConverterNameINSTANCE.Lower(participant), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *CompletionHandle
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterCompletionHandleINSTANCE.Lift(_uniffiRV), nil
	}
}

// Remove a participant and wait for completion (blocking version)
//
// This method removes a participant and blocks until the removal completes.
func (_self *BindingsSessionContext) RemoveAndWait(participant *Name) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_remove_and_wait(
			_pointer, FfiConverterNameINSTANCE.Lower(participant), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Remove a participant and wait for completion (async version)
//
// This method removes a participant and waits until the removal completes.
func (_self *BindingsSessionContext) RemoveAndWaitAsync(participant *Name) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_bindings_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_remove_and_wait_async(
			_pointer, FfiConverterNameINSTANCE.Lower(participant)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_void(handle)
		},
	)

	return err
}

// Remove a participant from the session (async version)
//
// Returns a completion handle that can be awaited to ensure the removal completes.
func (_self *BindingsSessionContext) RemoveAsync(participant *Name) (*CompletionHandle, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) unsafe.Pointer {
			res := C.ffi_slim_bindings_rust_future_complete_pointer(handle, status)
			return res
		},
		// liftFn
		func(ffi unsafe.Pointer) *CompletionHandle {
			return FfiConverterCompletionHandleINSTANCE.Lift(ffi)
		},
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_remove_async(
			_pointer, FfiConverterNameINSTANCE.Lower(participant)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_pointer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_pointer(handle)
		},
	)

	return res, err
}

// Get the session ID
func (_self *BindingsSessionContext) SessionId() (uint32, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) C.uint32_t {
		return C.uniffi_slim_bindings_fn_method_bindingssessioncontext_session_id(
			_pointer, _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue uint32
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterUint32INSTANCE.Lift(_uniffiRV), nil
	}
}

// Get the session type (PointToPoint or Group)
func (_self *BindingsSessionContext) SessionType() (SessionType, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_bindings_fn_method_bindingssessioncontext_session_type(
				_pointer, _uniffiStatus),
		}
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue SessionType
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterSessionTypeINSTANCE.Lift(_uniffiRV), nil
	}
}

// Get the source name for this session
func (_self *BindingsSessionContext) Source() (*Name, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_method_bindingssessioncontext_source(
			_pointer, _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *Name
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterNameINSTANCE.Lift(_uniffiRV), nil
	}
}
func (object *BindingsSessionContext) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterBindingsSessionContext struct{}

var FfiConverterBindingsSessionContextINSTANCE = FfiConverterBindingsSessionContext{}

func (c FfiConverterBindingsSessionContext) Lift(pointer unsafe.Pointer) *BindingsSessionContext {
	result := &BindingsSessionContext{
		newFfiObject(
			pointer,
			func(pointer unsafe.Pointer, status *C.RustCallStatus) unsafe.Pointer {
				return C.uniffi_slim_bindings_fn_clone_bindingssessioncontext(pointer, status)
			},
			func(pointer unsafe.Pointer, status *C.RustCallStatus) {
				C.uniffi_slim_bindings_fn_free_bindingssessioncontext(pointer, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*BindingsSessionContext).Destroy)
	return result
}

func (c FfiConverterBindingsSessionContext) Read(reader io.Reader) *BindingsSessionContext {
	return c.Lift(unsafe.Pointer(uintptr(readUint64(reader))))
}

func (c FfiConverterBindingsSessionContext) Lower(value *BindingsSessionContext) unsafe.Pointer {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the pointer will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked pointer.
	pointer := value.ffiObject.incrementPointer("*BindingsSessionContext")
	defer value.ffiObject.decrementPointer()
	return pointer

}

func (c FfiConverterBindingsSessionContext) Write(writer io.Writer, value *BindingsSessionContext) {
	writeUint64(writer, uint64(uintptr(c.Lower(value))))
}

type FfiDestroyerBindingsSessionContext struct{}

func (_ FfiDestroyerBindingsSessionContext) Destroy(value *BindingsSessionContext) {
	value.Destroy()
}

// FFI-compatible completion handle for async operations
//
// Represents a pending operation that can be awaited to ensure completion.
// Used for operations that need delivery confirmation or handshake acknowledgment.
//
// # Examples
//
// Basic usage:
// ```ignore
// let completion = session.publish(data, None, None)?;
// completion.wait()?; // Wait for delivery confirmation
// ```
type CompletionHandleInterface interface {
	// Wait for the operation to complete indefinitely (blocking version)
	//
	// This blocks the calling thread until the operation completes.
	// Use this from Go or other languages when you need to ensure
	// an operation has finished before proceeding.
	//
	// **Note:** This can only be called once per handle. Subsequent calls
	// will return an error.
	//
	// # Returns
	// * `Ok(())` - Operation completed successfully
	// * `Err(SlimError)` - Operation failed or handle already consumed
	Wait() error
	// Wait for the operation to complete indefinitely (async version)
	//
	// This is the async version that integrates with UniFFI's polling mechanism.
	// The operation will yield control while waiting.
	//
	// **Note:** This can only be called once per handle. Subsequent calls
	// will return an error.
	//
	// # Returns
	// * `Ok(())` - Operation completed successfully
	// * `Err(SlimError)` - Operation failed or handle already consumed
	WaitAsync() error
	// Wait for the operation to complete with a timeout (blocking version)
	//
	// This blocks the calling thread until the operation completes or the timeout expires.
	// Use this from Go or other languages when you need to ensure
	// an operation has finished before proceeding with a time limit.
	//
	// **Note:** This can only be called once per handle. Subsequent calls
	// will return an error.
	//
	// # Arguments
	// * `timeout` - Maximum time to wait for completion
	//
	// # Returns
	// * `Ok(())` - Operation completed successfully
	// * `Err(SlimError::Timeout)` - If the operation timed out
	// * `Err(SlimError)` - Operation failed or handle already consumed
	WaitFor(timeout time.Duration) error
	// Wait for the operation to complete with a timeout (async version)
	//
	// This is the async version that integrates with UniFFI's polling mechanism.
	// The operation will yield control while waiting until completion or timeout.
	//
	// **Note:** This can only be called once per handle. Subsequent calls
	// will return an error.
	//
	// # Arguments
	// * `timeout` - Maximum time to wait for completion
	//
	// # Returns
	// * `Ok(())` - Operation completed successfully
	// * `Err(SlimError::Timeout)` - If the operation timed out
	// * `Err(SlimError)` - Operation failed or handle already consumed
	WaitForAsync(timeout time.Duration) error
}

// FFI-compatible completion handle for async operations
//
// Represents a pending operation that can be awaited to ensure completion.
// Used for operations that need delivery confirmation or handshake acknowledgment.
//
// # Examples
//
// Basic usage:
// ```ignore
// let completion = session.publish(data, None, None)?;
// completion.wait()?; // Wait for delivery confirmation
// ```
type CompletionHandle struct {
	ffiObject FfiObject
}

// Wait for the operation to complete indefinitely (blocking version)
//
// This blocks the calling thread until the operation completes.
// Use this from Go or other languages when you need to ensure
// an operation has finished before proceeding.
//
// **Note:** This can only be called once per handle. Subsequent calls
// will return an error.
//
// # Returns
// * `Ok(())` - Operation completed successfully
// * `Err(SlimError)` - Operation failed or handle already consumed
func (_self *CompletionHandle) Wait() error {
	_pointer := _self.ffiObject.incrementPointer("*CompletionHandle")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_completionhandle_wait(
			_pointer, _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Wait for the operation to complete indefinitely (async version)
//
// This is the async version that integrates with UniFFI's polling mechanism.
// The operation will yield control while waiting.
//
// **Note:** This can only be called once per handle. Subsequent calls
// will return an error.
//
// # Returns
// * `Ok(())` - Operation completed successfully
// * `Err(SlimError)` - Operation failed or handle already consumed
func (_self *CompletionHandle) WaitAsync() error {
	_pointer := _self.ffiObject.incrementPointer("*CompletionHandle")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_bindings_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_bindings_fn_method_completionhandle_wait_async(
			_pointer),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_void(handle)
		},
	)

	return err
}

// Wait for the operation to complete with a timeout (blocking version)
//
// This blocks the calling thread until the operation completes or the timeout expires.
// Use this from Go or other languages when you need to ensure
// an operation has finished before proceeding with a time limit.
//
// **Note:** This can only be called once per handle. Subsequent calls
// will return an error.
//
// # Arguments
// * `timeout` - Maximum time to wait for completion
//
// # Returns
// * `Ok(())` - Operation completed successfully
// * `Err(SlimError::Timeout)` - If the operation timed out
// * `Err(SlimError)` - Operation failed or handle already consumed
func (_self *CompletionHandle) WaitFor(timeout time.Duration) error {
	_pointer := _self.ffiObject.incrementPointer("*CompletionHandle")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_completionhandle_wait_for(
			_pointer, FfiConverterDurationINSTANCE.Lower(timeout), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Wait for the operation to complete with a timeout (async version)
//
// This is the async version that integrates with UniFFI's polling mechanism.
// The operation will yield control while waiting until completion or timeout.
//
// **Note:** This can only be called once per handle. Subsequent calls
// will return an error.
//
// # Arguments
// * `timeout` - Maximum time to wait for completion
//
// # Returns
// * `Ok(())` - Operation completed successfully
// * `Err(SlimError::Timeout)` - If the operation timed out
// * `Err(SlimError)` - Operation failed or handle already consumed
func (_self *CompletionHandle) WaitForAsync(timeout time.Duration) error {
	_pointer := _self.ffiObject.incrementPointer("*CompletionHandle")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_bindings_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_bindings_fn_method_completionhandle_wait_for_async(
			_pointer, FfiConverterDurationINSTANCE.Lower(timeout)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_bindings_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_bindings_rust_future_free_void(handle)
		},
	)

	return err
}
func (object *CompletionHandle) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterCompletionHandle struct{}

var FfiConverterCompletionHandleINSTANCE = FfiConverterCompletionHandle{}

func (c FfiConverterCompletionHandle) Lift(pointer unsafe.Pointer) *CompletionHandle {
	result := &CompletionHandle{
		newFfiObject(
			pointer,
			func(pointer unsafe.Pointer, status *C.RustCallStatus) unsafe.Pointer {
				return C.uniffi_slim_bindings_fn_clone_completionhandle(pointer, status)
			},
			func(pointer unsafe.Pointer, status *C.RustCallStatus) {
				C.uniffi_slim_bindings_fn_free_completionhandle(pointer, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*CompletionHandle).Destroy)
	return result
}

func (c FfiConverterCompletionHandle) Read(reader io.Reader) *CompletionHandle {
	return c.Lift(unsafe.Pointer(uintptr(readUint64(reader))))
}

func (c FfiConverterCompletionHandle) Lower(value *CompletionHandle) unsafe.Pointer {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the pointer will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked pointer.
	pointer := value.ffiObject.incrementPointer("*CompletionHandle")
	defer value.ffiObject.decrementPointer()
	return pointer

}

func (c FfiConverterCompletionHandle) Write(writer io.Writer, value *CompletionHandle) {
	writeUint64(writer, uint64(uintptr(c.Lower(value))))
}

type FfiDestroyerCompletionHandle struct{}

func (_ FfiDestroyerCompletionHandle) Destroy(value *CompletionHandle) {
	value.Destroy()
}

// Name type for SLIM (Secure Low-Latency Interactive Messaging)
type NameInterface interface {
	// Get the name components as a vector of strings
	Components() []string
	// Get the name ID
	Id() uint64
}

// Name type for SLIM (Secure Low-Latency Interactive Messaging)
type Name struct {
	ffiObject FfiObject
}

// Create a new Name from a string
func NewName(component0 string, component1 string, component2 string, id *uint64) *Name {
	return FfiConverterNameINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_constructor_name_new(FfiConverterStringINSTANCE.Lower(component0), FfiConverterStringINSTANCE.Lower(component1), FfiConverterStringINSTANCE.Lower(component2), FfiConverterOptionalUint64INSTANCE.Lower(id), _uniffiStatus)
	}))
}

// Get the name components as a vector of strings
func (_self *Name) Components() []string {
	_pointer := _self.ffiObject.incrementPointer("*Name")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterSequenceStringINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_bindings_fn_method_name_components(
				_pointer, _uniffiStatus),
		}
	}))
}

// Get the name ID
func (_self *Name) Id() uint64 {
	_pointer := _self.ffiObject.incrementPointer("*Name")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterUint64INSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_slim_bindings_fn_method_name_id(
			_pointer, _uniffiStatus)
	}))
}
func (object *Name) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterName struct{}

var FfiConverterNameINSTANCE = FfiConverterName{}

func (c FfiConverterName) Lift(pointer unsafe.Pointer) *Name {
	result := &Name{
		newFfiObject(
			pointer,
			func(pointer unsafe.Pointer, status *C.RustCallStatus) unsafe.Pointer {
				return C.uniffi_slim_bindings_fn_clone_name(pointer, status)
			},
			func(pointer unsafe.Pointer, status *C.RustCallStatus) {
				C.uniffi_slim_bindings_fn_free_name(pointer, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*Name).Destroy)
	return result
}

func (c FfiConverterName) Read(reader io.Reader) *Name {
	return c.Lift(unsafe.Pointer(uintptr(readUint64(reader))))
}

func (c FfiConverterName) Lower(value *Name) unsafe.Pointer {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the pointer will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked pointer.
	pointer := value.ffiObject.incrementPointer("*Name")
	defer value.ffiObject.decrementPointer()
	return pointer

}

func (c FfiConverterName) Write(writer io.Writer, value *Name) {
	writeUint64(writer, uint64(uintptr(c.Lower(value))))
}

type FfiDestroyerName struct{}

func (_ FfiDestroyerName) Destroy(value *Name) {
	value.Destroy()
}

// Basic authentication configuration
type BasicAuth struct {
	Username string
	Password string
}

func (r *BasicAuth) Destroy() {
	FfiDestroyerString{}.Destroy(r.Username)
	FfiDestroyerString{}.Destroy(r.Password)
}

type FfiConverterBasicAuth struct{}

var FfiConverterBasicAuthINSTANCE = FfiConverterBasicAuth{}

func (c FfiConverterBasicAuth) Lift(rb RustBufferI) BasicAuth {
	return LiftFromRustBuffer[BasicAuth](c, rb)
}

func (c FfiConverterBasicAuth) Read(reader io.Reader) BasicAuth {
	return BasicAuth{
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterStringINSTANCE.Read(reader),
	}
}

func (c FfiConverterBasicAuth) Lower(value BasicAuth) C.RustBuffer {
	return LowerIntoRustBuffer[BasicAuth](c, value)
}

func (c FfiConverterBasicAuth) Write(writer io.Writer, value BasicAuth) {
	FfiConverterStringINSTANCE.Write(writer, value.Username)
	FfiConverterStringINSTANCE.Write(writer, value.Password)
}

type FfiDestroyerBasicAuth struct{}

func (_ FfiDestroyerBasicAuth) Destroy(value BasicAuth) {
	value.Destroy()
}

// Build information for the SLIM bindings
type BuildInfo struct {
	// Semantic version (e.g., "0.7.0")
	Version string
	// Git commit hash (short)
	GitSha string
	// Build date in ISO 8601 UTC format
	BuildDate string
	// Build profile (debug/release)
	Profile string
}

func (r *BuildInfo) Destroy() {
	FfiDestroyerString{}.Destroy(r.Version)
	FfiDestroyerString{}.Destroy(r.GitSha)
	FfiDestroyerString{}.Destroy(r.BuildDate)
	FfiDestroyerString{}.Destroy(r.Profile)
}

type FfiConverterBuildInfo struct{}

var FfiConverterBuildInfoINSTANCE = FfiConverterBuildInfo{}

func (c FfiConverterBuildInfo) Lift(rb RustBufferI) BuildInfo {
	return LiftFromRustBuffer[BuildInfo](c, rb)
}

func (c FfiConverterBuildInfo) Read(reader io.Reader) BuildInfo {
	return BuildInfo{
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterStringINSTANCE.Read(reader),
	}
}

func (c FfiConverterBuildInfo) Lower(value BuildInfo) C.RustBuffer {
	return LowerIntoRustBuffer[BuildInfo](c, value)
}

func (c FfiConverterBuildInfo) Write(writer io.Writer, value BuildInfo) {
	FfiConverterStringINSTANCE.Write(writer, value.Version)
	FfiConverterStringINSTANCE.Write(writer, value.GitSha)
	FfiConverterStringINSTANCE.Write(writer, value.BuildDate)
	FfiConverterStringINSTANCE.Write(writer, value.Profile)
}

type FfiDestroyerBuildInfo struct{}

func (_ FfiDestroyerBuildInfo) Destroy(value BuildInfo) {
	value.Destroy()
}

// Client configuration for connecting to a SLIM server
type ClientConfig struct {
	// The target endpoint the client will connect to
	Endpoint string
	// Origin (HTTP Host authority override) for the client
	Origin *string
	// Optional TLS SNI server name override
	ServerName *string
	// Compression type
	Compression *CompressionType
	// Rate limit string (e.g., "100/s" for 100 requests per second)
	RateLimit *string
	// TLS client configuration
	Tls TlsClientConfig
	// Keepalive parameters
	Keepalive *KeepaliveConfig
	// HTTP Proxy configuration
	Proxy ProxyConfig
	// Connection timeout
	ConnectTimeout time.Duration
	// Request timeout
	RequestTimeout time.Duration
	// Read buffer size in bytes
	BufferSize *uint64
	// Headers associated with gRPC requests
	Headers map[string]string
	// Authentication configuration for outgoing RPCs
	Auth ClientAuthenticationConfig
	// Backoff retry configuration
	Backoff BackoffConfig
	// Arbitrary user-provided metadata as JSON string
	Metadata *string
}

func (r *ClientConfig) Destroy() {
	FfiDestroyerString{}.Destroy(r.Endpoint)
	FfiDestroyerOptionalString{}.Destroy(r.Origin)
	FfiDestroyerOptionalString{}.Destroy(r.ServerName)
	FfiDestroyerOptionalCompressionType{}.Destroy(r.Compression)
	FfiDestroyerOptionalString{}.Destroy(r.RateLimit)
	FfiDestroyerTlsClientConfig{}.Destroy(r.Tls)
	FfiDestroyerOptionalKeepaliveConfig{}.Destroy(r.Keepalive)
	FfiDestroyerProxyConfig{}.Destroy(r.Proxy)
	FfiDestroyerDuration{}.Destroy(r.ConnectTimeout)
	FfiDestroyerDuration{}.Destroy(r.RequestTimeout)
	FfiDestroyerOptionalUint64{}.Destroy(r.BufferSize)
	FfiDestroyerMapStringString{}.Destroy(r.Headers)
	FfiDestroyerClientAuthenticationConfig{}.Destroy(r.Auth)
	FfiDestroyerBackoffConfig{}.Destroy(r.Backoff)
	FfiDestroyerOptionalString{}.Destroy(r.Metadata)
}

type FfiConverterClientConfig struct{}

var FfiConverterClientConfigINSTANCE = FfiConverterClientConfig{}

func (c FfiConverterClientConfig) Lift(rb RustBufferI) ClientConfig {
	return LiftFromRustBuffer[ClientConfig](c, rb)
}

func (c FfiConverterClientConfig) Read(reader io.Reader) ClientConfig {
	return ClientConfig{
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterOptionalCompressionTypeINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterTlsClientConfigINSTANCE.Read(reader),
		FfiConverterOptionalKeepaliveConfigINSTANCE.Read(reader),
		FfiConverterProxyConfigINSTANCE.Read(reader),
		FfiConverterDurationINSTANCE.Read(reader),
		FfiConverterDurationINSTANCE.Read(reader),
		FfiConverterOptionalUint64INSTANCE.Read(reader),
		FfiConverterMapStringStringINSTANCE.Read(reader),
		FfiConverterClientAuthenticationConfigINSTANCE.Read(reader),
		FfiConverterBackoffConfigINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
	}
}

func (c FfiConverterClientConfig) Lower(value ClientConfig) C.RustBuffer {
	return LowerIntoRustBuffer[ClientConfig](c, value)
}

func (c FfiConverterClientConfig) Write(writer io.Writer, value ClientConfig) {
	FfiConverterStringINSTANCE.Write(writer, value.Endpoint)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.Origin)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.ServerName)
	FfiConverterOptionalCompressionTypeINSTANCE.Write(writer, value.Compression)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.RateLimit)
	FfiConverterTlsClientConfigINSTANCE.Write(writer, value.Tls)
	FfiConverterOptionalKeepaliveConfigINSTANCE.Write(writer, value.Keepalive)
	FfiConverterProxyConfigINSTANCE.Write(writer, value.Proxy)
	FfiConverterDurationINSTANCE.Write(writer, value.ConnectTimeout)
	FfiConverterDurationINSTANCE.Write(writer, value.RequestTimeout)
	FfiConverterOptionalUint64INSTANCE.Write(writer, value.BufferSize)
	FfiConverterMapStringStringINSTANCE.Write(writer, value.Headers)
	FfiConverterClientAuthenticationConfigINSTANCE.Write(writer, value.Auth)
	FfiConverterBackoffConfigINSTANCE.Write(writer, value.Backoff)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.Metadata)
}

type FfiDestroyerClientConfig struct{}

func (_ FfiDestroyerClientConfig) Destroy(value ClientConfig) {
	value.Destroy()
}

// JWT authentication configuration for client-side signing
type ClientJwtAuth struct {
	// JWT key configuration (encoding key for signing)
	Key JwtKeyType
	// JWT audience claims to include
	Audience *[]string
	// JWT issuer to include
	Issuer *string
	// JWT subject to include
	Subject *string
	// Token validity duration (default: 3600 seconds)
	Duration time.Duration
}

func (r *ClientJwtAuth) Destroy() {
	FfiDestroyerJwtKeyType{}.Destroy(r.Key)
	FfiDestroyerOptionalSequenceString{}.Destroy(r.Audience)
	FfiDestroyerOptionalString{}.Destroy(r.Issuer)
	FfiDestroyerOptionalString{}.Destroy(r.Subject)
	FfiDestroyerDuration{}.Destroy(r.Duration)
}

type FfiConverterClientJwtAuth struct{}

var FfiConverterClientJwtAuthINSTANCE = FfiConverterClientJwtAuth{}

func (c FfiConverterClientJwtAuth) Lift(rb RustBufferI) ClientJwtAuth {
	return LiftFromRustBuffer[ClientJwtAuth](c, rb)
}

func (c FfiConverterClientJwtAuth) Read(reader io.Reader) ClientJwtAuth {
	return ClientJwtAuth{
		FfiConverterJwtKeyTypeINSTANCE.Read(reader),
		FfiConverterOptionalSequenceStringINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterDurationINSTANCE.Read(reader),
	}
}

func (c FfiConverterClientJwtAuth) Lower(value ClientJwtAuth) C.RustBuffer {
	return LowerIntoRustBuffer[ClientJwtAuth](c, value)
}

func (c FfiConverterClientJwtAuth) Write(writer io.Writer, value ClientJwtAuth) {
	FfiConverterJwtKeyTypeINSTANCE.Write(writer, value.Key)
	FfiConverterOptionalSequenceStringINSTANCE.Write(writer, value.Audience)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.Issuer)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.Subject)
	FfiConverterDurationINSTANCE.Write(writer, value.Duration)
}

type FfiDestroyerClientJwtAuth struct{}

func (_ FfiDestroyerClientJwtAuth) Destroy(value ClientJwtAuth) {
	value.Destroy()
}

// Exponential backoff configuration
type ExponentialBackoff struct {
	// Base delay
	Base time.Duration
	// Multiplication factor for each retry
	Factor uint64
	// Maximum delay
	MaxDelay time.Duration
	// Maximum number of retry attempts
	MaxAttempts uint64
	// Whether to add random jitter to delays
	Jitter bool
}

func (r *ExponentialBackoff) Destroy() {
	FfiDestroyerDuration{}.Destroy(r.Base)
	FfiDestroyerUint64{}.Destroy(r.Factor)
	FfiDestroyerDuration{}.Destroy(r.MaxDelay)
	FfiDestroyerUint64{}.Destroy(r.MaxAttempts)
	FfiDestroyerBool{}.Destroy(r.Jitter)
}

type FfiConverterExponentialBackoff struct{}

var FfiConverterExponentialBackoffINSTANCE = FfiConverterExponentialBackoff{}

func (c FfiConverterExponentialBackoff) Lift(rb RustBufferI) ExponentialBackoff {
	return LiftFromRustBuffer[ExponentialBackoff](c, rb)
}

func (c FfiConverterExponentialBackoff) Read(reader io.Reader) ExponentialBackoff {
	return ExponentialBackoff{
		FfiConverterDurationINSTANCE.Read(reader),
		FfiConverterUint64INSTANCE.Read(reader),
		FfiConverterDurationINSTANCE.Read(reader),
		FfiConverterUint64INSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
	}
}

func (c FfiConverterExponentialBackoff) Lower(value ExponentialBackoff) C.RustBuffer {
	return LowerIntoRustBuffer[ExponentialBackoff](c, value)
}

func (c FfiConverterExponentialBackoff) Write(writer io.Writer, value ExponentialBackoff) {
	FfiConverterDurationINSTANCE.Write(writer, value.Base)
	FfiConverterUint64INSTANCE.Write(writer, value.Factor)
	FfiConverterDurationINSTANCE.Write(writer, value.MaxDelay)
	FfiConverterUint64INSTANCE.Write(writer, value.MaxAttempts)
	FfiConverterBoolINSTANCE.Write(writer, value.Jitter)
}

type FfiDestroyerExponentialBackoff struct{}

func (_ FfiDestroyerExponentialBackoff) Destroy(value ExponentialBackoff) {
	value.Destroy()
}

// Fixed interval backoff configuration
type FixedIntervalBackoff struct {
	// Fixed interval between retries
	Interval time.Duration
	// Maximum number of retry attempts
	MaxAttempts uint64
}

func (r *FixedIntervalBackoff) Destroy() {
	FfiDestroyerDuration{}.Destroy(r.Interval)
	FfiDestroyerUint64{}.Destroy(r.MaxAttempts)
}

type FfiConverterFixedIntervalBackoff struct{}

var FfiConverterFixedIntervalBackoffINSTANCE = FfiConverterFixedIntervalBackoff{}

func (c FfiConverterFixedIntervalBackoff) Lift(rb RustBufferI) FixedIntervalBackoff {
	return LiftFromRustBuffer[FixedIntervalBackoff](c, rb)
}

func (c FfiConverterFixedIntervalBackoff) Read(reader io.Reader) FixedIntervalBackoff {
	return FixedIntervalBackoff{
		FfiConverterDurationINSTANCE.Read(reader),
		FfiConverterUint64INSTANCE.Read(reader),
	}
}

func (c FfiConverterFixedIntervalBackoff) Lower(value FixedIntervalBackoff) C.RustBuffer {
	return LowerIntoRustBuffer[FixedIntervalBackoff](c, value)
}

func (c FfiConverterFixedIntervalBackoff) Write(writer io.Writer, value FixedIntervalBackoff) {
	FfiConverterDurationINSTANCE.Write(writer, value.Interval)
	FfiConverterUint64INSTANCE.Write(writer, value.MaxAttempts)
}

type FfiDestroyerFixedIntervalBackoff struct{}

func (_ FfiDestroyerFixedIntervalBackoff) Destroy(value FixedIntervalBackoff) {
	value.Destroy()
}

// JWT authentication configuration for server-side verification
type JwtAuth struct {
	// JWT key configuration (decoding key for verification)
	Key JwtKeyType
	// JWT audience claims to verify
	Audience *[]string
	// JWT issuer to verify
	Issuer *string
	// JWT subject to verify
	Subject *string
	// Token validity duration (default: 3600 seconds)
	Duration time.Duration
}

func (r *JwtAuth) Destroy() {
	FfiDestroyerJwtKeyType{}.Destroy(r.Key)
	FfiDestroyerOptionalSequenceString{}.Destroy(r.Audience)
	FfiDestroyerOptionalString{}.Destroy(r.Issuer)
	FfiDestroyerOptionalString{}.Destroy(r.Subject)
	FfiDestroyerDuration{}.Destroy(r.Duration)
}

type FfiConverterJwtAuth struct{}

var FfiConverterJwtAuthINSTANCE = FfiConverterJwtAuth{}

func (c FfiConverterJwtAuth) Lift(rb RustBufferI) JwtAuth {
	return LiftFromRustBuffer[JwtAuth](c, rb)
}

func (c FfiConverterJwtAuth) Read(reader io.Reader) JwtAuth {
	return JwtAuth{
		FfiConverterJwtKeyTypeINSTANCE.Read(reader),
		FfiConverterOptionalSequenceStringINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterDurationINSTANCE.Read(reader),
	}
}

func (c FfiConverterJwtAuth) Lower(value JwtAuth) C.RustBuffer {
	return LowerIntoRustBuffer[JwtAuth](c, value)
}

func (c FfiConverterJwtAuth) Write(writer io.Writer, value JwtAuth) {
	FfiConverterJwtKeyTypeINSTANCE.Write(writer, value.Key)
	FfiConverterOptionalSequenceStringINSTANCE.Write(writer, value.Audience)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.Issuer)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.Subject)
	FfiConverterDurationINSTANCE.Write(writer, value.Duration)
}

type FfiDestroyerJwtAuth struct{}

func (_ FfiDestroyerJwtAuth) Destroy(value JwtAuth) {
	value.Destroy()
}

// JWT key configuration
type JwtKeyConfig struct {
	// Algorithm used for signing/verifying the JWT
	Algorithm JwtAlgorithm
	// Key format - PEM, JWK or JWKS
	Format JwtKeyFormat
	// Encoded key or file path
	Key JwtKeyData
}

func (r *JwtKeyConfig) Destroy() {
	FfiDestroyerJwtAlgorithm{}.Destroy(r.Algorithm)
	FfiDestroyerJwtKeyFormat{}.Destroy(r.Format)
	FfiDestroyerJwtKeyData{}.Destroy(r.Key)
}

type FfiConverterJwtKeyConfig struct{}

var FfiConverterJwtKeyConfigINSTANCE = FfiConverterJwtKeyConfig{}

func (c FfiConverterJwtKeyConfig) Lift(rb RustBufferI) JwtKeyConfig {
	return LiftFromRustBuffer[JwtKeyConfig](c, rb)
}

func (c FfiConverterJwtKeyConfig) Read(reader io.Reader) JwtKeyConfig {
	return JwtKeyConfig{
		FfiConverterJwtAlgorithmINSTANCE.Read(reader),
		FfiConverterJwtKeyFormatINSTANCE.Read(reader),
		FfiConverterJwtKeyDataINSTANCE.Read(reader),
	}
}

func (c FfiConverterJwtKeyConfig) Lower(value JwtKeyConfig) C.RustBuffer {
	return LowerIntoRustBuffer[JwtKeyConfig](c, value)
}

func (c FfiConverterJwtKeyConfig) Write(writer io.Writer, value JwtKeyConfig) {
	FfiConverterJwtAlgorithmINSTANCE.Write(writer, value.Algorithm)
	FfiConverterJwtKeyFormatINSTANCE.Write(writer, value.Format)
	FfiConverterJwtKeyDataINSTANCE.Write(writer, value.Key)
}

type FfiDestroyerJwtKeyConfig struct{}

func (_ FfiDestroyerJwtKeyConfig) Destroy(value JwtKeyConfig) {
	value.Destroy()
}

// Keepalive configuration for the client
type KeepaliveConfig struct {
	// TCP keepalive duration
	TcpKeepalive time.Duration
	// HTTP2 keepalive duration
	Http2Keepalive time.Duration
	// Keepalive timeout
	Timeout time.Duration
	// Whether to permit keepalive without an active stream
	KeepAliveWhileIdle bool
}

func (r *KeepaliveConfig) Destroy() {
	FfiDestroyerDuration{}.Destroy(r.TcpKeepalive)
	FfiDestroyerDuration{}.Destroy(r.Http2Keepalive)
	FfiDestroyerDuration{}.Destroy(r.Timeout)
	FfiDestroyerBool{}.Destroy(r.KeepAliveWhileIdle)
}

type FfiConverterKeepaliveConfig struct{}

var FfiConverterKeepaliveConfigINSTANCE = FfiConverterKeepaliveConfig{}

func (c FfiConverterKeepaliveConfig) Lift(rb RustBufferI) KeepaliveConfig {
	return LiftFromRustBuffer[KeepaliveConfig](c, rb)
}

func (c FfiConverterKeepaliveConfig) Read(reader io.Reader) KeepaliveConfig {
	return KeepaliveConfig{
		FfiConverterDurationINSTANCE.Read(reader),
		FfiConverterDurationINSTANCE.Read(reader),
		FfiConverterDurationINSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
	}
}

func (c FfiConverterKeepaliveConfig) Lower(value KeepaliveConfig) C.RustBuffer {
	return LowerIntoRustBuffer[KeepaliveConfig](c, value)
}

func (c FfiConverterKeepaliveConfig) Write(writer io.Writer, value KeepaliveConfig) {
	FfiConverterDurationINSTANCE.Write(writer, value.TcpKeepalive)
	FfiConverterDurationINSTANCE.Write(writer, value.Http2Keepalive)
	FfiConverterDurationINSTANCE.Write(writer, value.Timeout)
	FfiConverterBoolINSTANCE.Write(writer, value.KeepAliveWhileIdle)
}

type FfiDestroyerKeepaliveConfig struct{}

func (_ FfiDestroyerKeepaliveConfig) Destroy(value KeepaliveConfig) {
	value.Destroy()
}

// Keepalive configuration for the server
type KeepaliveServerParameters struct {
	// Max connection idle time (time after which an idle connection is closed)
	MaxConnectionIdle time.Duration
	// Max connection age (maximum time a connection may exist before being closed)
	MaxConnectionAge time.Duration
	// Max connection age grace (additional time after max_connection_age before closing)
	MaxConnectionAgeGrace time.Duration
	// Keepalive ping frequency
	Time time.Duration
	// Keepalive ping timeout (time to wait for ack)
	Timeout time.Duration
}

func (r *KeepaliveServerParameters) Destroy() {
	FfiDestroyerDuration{}.Destroy(r.MaxConnectionIdle)
	FfiDestroyerDuration{}.Destroy(r.MaxConnectionAge)
	FfiDestroyerDuration{}.Destroy(r.MaxConnectionAgeGrace)
	FfiDestroyerDuration{}.Destroy(r.Time)
	FfiDestroyerDuration{}.Destroy(r.Timeout)
}

type FfiConverterKeepaliveServerParameters struct{}

var FfiConverterKeepaliveServerParametersINSTANCE = FfiConverterKeepaliveServerParameters{}

func (c FfiConverterKeepaliveServerParameters) Lift(rb RustBufferI) KeepaliveServerParameters {
	return LiftFromRustBuffer[KeepaliveServerParameters](c, rb)
}

func (c FfiConverterKeepaliveServerParameters) Read(reader io.Reader) KeepaliveServerParameters {
	return KeepaliveServerParameters{
		FfiConverterDurationINSTANCE.Read(reader),
		FfiConverterDurationINSTANCE.Read(reader),
		FfiConverterDurationINSTANCE.Read(reader),
		FfiConverterDurationINSTANCE.Read(reader),
		FfiConverterDurationINSTANCE.Read(reader),
	}
}

func (c FfiConverterKeepaliveServerParameters) Lower(value KeepaliveServerParameters) C.RustBuffer {
	return LowerIntoRustBuffer[KeepaliveServerParameters](c, value)
}

func (c FfiConverterKeepaliveServerParameters) Write(writer io.Writer, value KeepaliveServerParameters) {
	FfiConverterDurationINSTANCE.Write(writer, value.MaxConnectionIdle)
	FfiConverterDurationINSTANCE.Write(writer, value.MaxConnectionAge)
	FfiConverterDurationINSTANCE.Write(writer, value.MaxConnectionAgeGrace)
	FfiConverterDurationINSTANCE.Write(writer, value.Time)
	FfiConverterDurationINSTANCE.Write(writer, value.Timeout)
}

type FfiDestroyerKeepaliveServerParameters struct{}

func (_ FfiDestroyerKeepaliveServerParameters) Destroy(value KeepaliveServerParameters) {
	value.Destroy()
}

// Generic message context for language bindings (UniFFI-compatible)
//
// Provides routing and descriptive metadata needed for replying,
// auditing, and instrumentation across different language bindings.
// This type is exported to foreign languages via UniFFI.
type MessageContext struct {
	// Fully-qualified sender identity
	SourceName *Name
	// Fully-qualified destination identity (may be empty for broadcast/group scenarios)
	DestinationName **Name
	// Logical/semantic type (defaults to "msg" if unspecified)
	PayloadType string
	// Arbitrary key/value pairs supplied by the sender (e.g. tracing IDs)
	Metadata map[string]string
	// Numeric identifier of the inbound connection carrying the message
	InputConnection uint64
	// Identity contained in the message
	Identity string
}

func (r *MessageContext) Destroy() {
	FfiDestroyerName{}.Destroy(r.SourceName)
	FfiDestroyerOptionalName{}.Destroy(r.DestinationName)
	FfiDestroyerString{}.Destroy(r.PayloadType)
	FfiDestroyerMapStringString{}.Destroy(r.Metadata)
	FfiDestroyerUint64{}.Destroy(r.InputConnection)
	FfiDestroyerString{}.Destroy(r.Identity)
}

type FfiConverterMessageContext struct{}

var FfiConverterMessageContextINSTANCE = FfiConverterMessageContext{}

func (c FfiConverterMessageContext) Lift(rb RustBufferI) MessageContext {
	return LiftFromRustBuffer[MessageContext](c, rb)
}

func (c FfiConverterMessageContext) Read(reader io.Reader) MessageContext {
	return MessageContext{
		FfiConverterNameINSTANCE.Read(reader),
		FfiConverterOptionalNameINSTANCE.Read(reader),
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterMapStringStringINSTANCE.Read(reader),
		FfiConverterUint64INSTANCE.Read(reader),
		FfiConverterStringINSTANCE.Read(reader),
	}
}

func (c FfiConverterMessageContext) Lower(value MessageContext) C.RustBuffer {
	return LowerIntoRustBuffer[MessageContext](c, value)
}

func (c FfiConverterMessageContext) Write(writer io.Writer, value MessageContext) {
	FfiConverterNameINSTANCE.Write(writer, value.SourceName)
	FfiConverterOptionalNameINSTANCE.Write(writer, value.DestinationName)
	FfiConverterStringINSTANCE.Write(writer, value.PayloadType)
	FfiConverterMapStringStringINSTANCE.Write(writer, value.Metadata)
	FfiConverterUint64INSTANCE.Write(writer, value.InputConnection)
	FfiConverterStringINSTANCE.Write(writer, value.Identity)
}

type FfiDestroyerMessageContext struct{}

func (_ FfiDestroyerMessageContext) Destroy(value MessageContext) {
	value.Destroy()
}

// HTTP Proxy configuration
type ProxyConfig struct {
	// The HTTP proxy URL (e.g., "http://proxy.example.com:8080")
	Url *string
	// TLS configuration for proxy connection
	Tls TlsClientConfig
	// Optional username for proxy authentication
	Username *string
	// Optional password for proxy authentication
	Password *string
	// Headers to send with proxy requests
	Headers map[string]string
}

func (r *ProxyConfig) Destroy() {
	FfiDestroyerOptionalString{}.Destroy(r.Url)
	FfiDestroyerTlsClientConfig{}.Destroy(r.Tls)
	FfiDestroyerOptionalString{}.Destroy(r.Username)
	FfiDestroyerOptionalString{}.Destroy(r.Password)
	FfiDestroyerMapStringString{}.Destroy(r.Headers)
}

type FfiConverterProxyConfig struct{}

var FfiConverterProxyConfigINSTANCE = FfiConverterProxyConfig{}

func (c FfiConverterProxyConfig) Lift(rb RustBufferI) ProxyConfig {
	return LiftFromRustBuffer[ProxyConfig](c, rb)
}

func (c FfiConverterProxyConfig) Read(reader io.Reader) ProxyConfig {
	return ProxyConfig{
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterTlsClientConfigINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterMapStringStringINSTANCE.Read(reader),
	}
}

func (c FfiConverterProxyConfig) Lower(value ProxyConfig) C.RustBuffer {
	return LowerIntoRustBuffer[ProxyConfig](c, value)
}

func (c FfiConverterProxyConfig) Write(writer io.Writer, value ProxyConfig) {
	FfiConverterOptionalStringINSTANCE.Write(writer, value.Url)
	FfiConverterTlsClientConfigINSTANCE.Write(writer, value.Tls)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.Username)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.Password)
	FfiConverterMapStringStringINSTANCE.Write(writer, value.Headers)
}

type FfiDestroyerProxyConfig struct{}

func (_ FfiDestroyerProxyConfig) Destroy(value ProxyConfig) {
	value.Destroy()
}

// Received message containing context and payload
type ReceivedMessage struct {
	Context MessageContext
	Payload []byte
}

func (r *ReceivedMessage) Destroy() {
	FfiDestroyerMessageContext{}.Destroy(r.Context)
	FfiDestroyerBytes{}.Destroy(r.Payload)
}

type FfiConverterReceivedMessage struct{}

var FfiConverterReceivedMessageINSTANCE = FfiConverterReceivedMessage{}

func (c FfiConverterReceivedMessage) Lift(rb RustBufferI) ReceivedMessage {
	return LiftFromRustBuffer[ReceivedMessage](c, rb)
}

func (c FfiConverterReceivedMessage) Read(reader io.Reader) ReceivedMessage {
	return ReceivedMessage{
		FfiConverterMessageContextINSTANCE.Read(reader),
		FfiConverterBytesINSTANCE.Read(reader),
	}
}

func (c FfiConverterReceivedMessage) Lower(value ReceivedMessage) C.RustBuffer {
	return LowerIntoRustBuffer[ReceivedMessage](c, value)
}

func (c FfiConverterReceivedMessage) Write(writer io.Writer, value ReceivedMessage) {
	FfiConverterMessageContextINSTANCE.Write(writer, value.Context)
	FfiConverterBytesINSTANCE.Write(writer, value.Payload)
}

type FfiDestroyerReceivedMessage struct{}

func (_ FfiDestroyerReceivedMessage) Destroy(value ReceivedMessage) {
	value.Destroy()
}

// Server configuration for running a SLIM server
type ServerConfig struct {
	// Endpoint address to listen on (e.g., "0.0.0.0:50051" or "[::]:50051")
	Endpoint string
	// TLS server configuration
	Tls TlsServerConfig
	// Use HTTP/2 only (default: true)
	Http2Only bool
	// Maximum size (in MiB) of messages accepted by the server
	MaxFrameSize *uint32
	// Maximum number of concurrent streams per connection
	MaxConcurrentStreams *uint32
	// Maximum header list size in bytes
	MaxHeaderListSize *uint32
	// Read buffer size in bytes
	ReadBufferSize *uint64
	// Write buffer size in bytes
	WriteBufferSize *uint64
	// Keepalive parameters
	Keepalive KeepaliveServerParameters
	// Authentication configuration for incoming requests
	Auth ServerAuthenticationConfig
	// Arbitrary user-provided metadata as JSON string
	Metadata *string
}

func (r *ServerConfig) Destroy() {
	FfiDestroyerString{}.Destroy(r.Endpoint)
	FfiDestroyerTlsServerConfig{}.Destroy(r.Tls)
	FfiDestroyerBool{}.Destroy(r.Http2Only)
	FfiDestroyerOptionalUint32{}.Destroy(r.MaxFrameSize)
	FfiDestroyerOptionalUint32{}.Destroy(r.MaxConcurrentStreams)
	FfiDestroyerOptionalUint32{}.Destroy(r.MaxHeaderListSize)
	FfiDestroyerOptionalUint64{}.Destroy(r.ReadBufferSize)
	FfiDestroyerOptionalUint64{}.Destroy(r.WriteBufferSize)
	FfiDestroyerKeepaliveServerParameters{}.Destroy(r.Keepalive)
	FfiDestroyerServerAuthenticationConfig{}.Destroy(r.Auth)
	FfiDestroyerOptionalString{}.Destroy(r.Metadata)
}

type FfiConverterServerConfig struct{}

var FfiConverterServerConfigINSTANCE = FfiConverterServerConfig{}

func (c FfiConverterServerConfig) Lift(rb RustBufferI) ServerConfig {
	return LiftFromRustBuffer[ServerConfig](c, rb)
}

func (c FfiConverterServerConfig) Read(reader io.Reader) ServerConfig {
	return ServerConfig{
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterTlsServerConfigINSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
		FfiConverterOptionalUint32INSTANCE.Read(reader),
		FfiConverterOptionalUint32INSTANCE.Read(reader),
		FfiConverterOptionalUint32INSTANCE.Read(reader),
		FfiConverterOptionalUint64INSTANCE.Read(reader),
		FfiConverterOptionalUint64INSTANCE.Read(reader),
		FfiConverterKeepaliveServerParametersINSTANCE.Read(reader),
		FfiConverterServerAuthenticationConfigINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
	}
}

func (c FfiConverterServerConfig) Lower(value ServerConfig) C.RustBuffer {
	return LowerIntoRustBuffer[ServerConfig](c, value)
}

func (c FfiConverterServerConfig) Write(writer io.Writer, value ServerConfig) {
	FfiConverterStringINSTANCE.Write(writer, value.Endpoint)
	FfiConverterTlsServerConfigINSTANCE.Write(writer, value.Tls)
	FfiConverterBoolINSTANCE.Write(writer, value.Http2Only)
	FfiConverterOptionalUint32INSTANCE.Write(writer, value.MaxFrameSize)
	FfiConverterOptionalUint32INSTANCE.Write(writer, value.MaxConcurrentStreams)
	FfiConverterOptionalUint32INSTANCE.Write(writer, value.MaxHeaderListSize)
	FfiConverterOptionalUint64INSTANCE.Write(writer, value.ReadBufferSize)
	FfiConverterOptionalUint64INSTANCE.Write(writer, value.WriteBufferSize)
	FfiConverterKeepaliveServerParametersINSTANCE.Write(writer, value.Keepalive)
	FfiConverterServerAuthenticationConfigINSTANCE.Write(writer, value.Auth)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.Metadata)
}

type FfiDestroyerServerConfig struct{}

func (_ FfiDestroyerServerConfig) Destroy(value ServerConfig) {
	value.Destroy()
}

// Session configuration
type SessionConfig struct {
	// Session type (PointToPoint or Group)
	SessionType SessionType
	// Enable MLS encryption for this session
	EnableMls bool
	// Maximum number of retries for message transmission (None = use default)
	MaxRetries *uint32
	// Interval between retries in milliseconds (None = use default)
	Interval *time.Duration
	// Custom metadata key-value pairs for the session
	Metadata map[string]string
}

func (r *SessionConfig) Destroy() {
	FfiDestroyerSessionType{}.Destroy(r.SessionType)
	FfiDestroyerBool{}.Destroy(r.EnableMls)
	FfiDestroyerOptionalUint32{}.Destroy(r.MaxRetries)
	FfiDestroyerOptionalDuration{}.Destroy(r.Interval)
	FfiDestroyerMapStringString{}.Destroy(r.Metadata)
}

type FfiConverterSessionConfig struct{}

var FfiConverterSessionConfigINSTANCE = FfiConverterSessionConfig{}

func (c FfiConverterSessionConfig) Lift(rb RustBufferI) SessionConfig {
	return LiftFromRustBuffer[SessionConfig](c, rb)
}

func (c FfiConverterSessionConfig) Read(reader io.Reader) SessionConfig {
	return SessionConfig{
		FfiConverterSessionTypeINSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
		FfiConverterOptionalUint32INSTANCE.Read(reader),
		FfiConverterOptionalDurationINSTANCE.Read(reader),
		FfiConverterMapStringStringINSTANCE.Read(reader),
	}
}

func (c FfiConverterSessionConfig) Lower(value SessionConfig) C.RustBuffer {
	return LowerIntoRustBuffer[SessionConfig](c, value)
}

func (c FfiConverterSessionConfig) Write(writer io.Writer, value SessionConfig) {
	FfiConverterSessionTypeINSTANCE.Write(writer, value.SessionType)
	FfiConverterBoolINSTANCE.Write(writer, value.EnableMls)
	FfiConverterOptionalUint32INSTANCE.Write(writer, value.MaxRetries)
	FfiConverterOptionalDurationINSTANCE.Write(writer, value.Interval)
	FfiConverterMapStringStringINSTANCE.Write(writer, value.Metadata)
}

type FfiDestroyerSessionConfig struct{}

func (_ FfiDestroyerSessionConfig) Destroy(value SessionConfig) {
	value.Destroy()
}

// Result of creating a session, containing the session context and a completion handle
//
// The completion handle should be awaited to ensure the session is fully established.
type SessionWithCompletion struct {
	// The session context for performing operations
	Session *BindingsSessionContext
	// Completion handle to wait for session establishment
	Completion *CompletionHandle
}

func (r *SessionWithCompletion) Destroy() {
	FfiDestroyerBindingsSessionContext{}.Destroy(r.Session)
	FfiDestroyerCompletionHandle{}.Destroy(r.Completion)
}

type FfiConverterSessionWithCompletion struct{}

var FfiConverterSessionWithCompletionINSTANCE = FfiConverterSessionWithCompletion{}

func (c FfiConverterSessionWithCompletion) Lift(rb RustBufferI) SessionWithCompletion {
	return LiftFromRustBuffer[SessionWithCompletion](c, rb)
}

func (c FfiConverterSessionWithCompletion) Read(reader io.Reader) SessionWithCompletion {
	return SessionWithCompletion{
		FfiConverterBindingsSessionContextINSTANCE.Read(reader),
		FfiConverterCompletionHandleINSTANCE.Read(reader),
	}
}

func (c FfiConverterSessionWithCompletion) Lower(value SessionWithCompletion) C.RustBuffer {
	return LowerIntoRustBuffer[SessionWithCompletion](c, value)
}

func (c FfiConverterSessionWithCompletion) Write(writer io.Writer, value SessionWithCompletion) {
	FfiConverterBindingsSessionContextINSTANCE.Write(writer, value.Session)
	FfiConverterCompletionHandleINSTANCE.Write(writer, value.Completion)
}

type FfiDestroyerSessionWithCompletion struct{}

func (_ FfiDestroyerSessionWithCompletion) Destroy(value SessionWithCompletion) {
	value.Destroy()
}

// SPIRE configuration for SPIFFE Workload API integration
type SpireConfig struct {
	// Path to the SPIFFE Workload API socket (None => use SPIFFE_ENDPOINT_SOCKET env var)
	SocketPath *string
	// Optional target SPIFFE ID when requesting JWT SVIDs
	TargetSpiffeId *string
	// Audiences to request/verify for JWT SVIDs
	JwtAudiences []string
	// Optional trust domains override for X.509 bundle retrieval
	TrustDomains []string
}

func (r *SpireConfig) Destroy() {
	FfiDestroyerOptionalString{}.Destroy(r.SocketPath)
	FfiDestroyerOptionalString{}.Destroy(r.TargetSpiffeId)
	FfiDestroyerSequenceString{}.Destroy(r.JwtAudiences)
	FfiDestroyerSequenceString{}.Destroy(r.TrustDomains)
}

type FfiConverterSpireConfig struct{}

var FfiConverterSpireConfigINSTANCE = FfiConverterSpireConfig{}

func (c FfiConverterSpireConfig) Lift(rb RustBufferI) SpireConfig {
	return LiftFromRustBuffer[SpireConfig](c, rb)
}

func (c FfiConverterSpireConfig) Read(reader io.Reader) SpireConfig {
	return SpireConfig{
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterSequenceStringINSTANCE.Read(reader),
		FfiConverterSequenceStringINSTANCE.Read(reader),
	}
}

func (c FfiConverterSpireConfig) Lower(value SpireConfig) C.RustBuffer {
	return LowerIntoRustBuffer[SpireConfig](c, value)
}

func (c FfiConverterSpireConfig) Write(writer io.Writer, value SpireConfig) {
	FfiConverterOptionalStringINSTANCE.Write(writer, value.SocketPath)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.TargetSpiffeId)
	FfiConverterSequenceStringINSTANCE.Write(writer, value.JwtAudiences)
	FfiConverterSequenceStringINSTANCE.Write(writer, value.TrustDomains)
}

type FfiDestroyerSpireConfig struct{}

func (_ FfiDestroyerSpireConfig) Destroy(value SpireConfig) {
	value.Destroy()
}

// Static JWT (Bearer token) authentication configuration
// The token is loaded from a file and automatically reloaded when changed
type StaticJwtAuth struct {
	// Path to file containing the JWT token
	TokenFile string
	// Duration for caching the token before re-reading from file (default: 3600 seconds)
	Duration time.Duration
}

func (r *StaticJwtAuth) Destroy() {
	FfiDestroyerString{}.Destroy(r.TokenFile)
	FfiDestroyerDuration{}.Destroy(r.Duration)
}

type FfiConverterStaticJwtAuth struct{}

var FfiConverterStaticJwtAuthINSTANCE = FfiConverterStaticJwtAuth{}

func (c FfiConverterStaticJwtAuth) Lift(rb RustBufferI) StaticJwtAuth {
	return LiftFromRustBuffer[StaticJwtAuth](c, rb)
}

func (c FfiConverterStaticJwtAuth) Read(reader io.Reader) StaticJwtAuth {
	return StaticJwtAuth{
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterDurationINSTANCE.Read(reader),
	}
}

func (c FfiConverterStaticJwtAuth) Lower(value StaticJwtAuth) C.RustBuffer {
	return LowerIntoRustBuffer[StaticJwtAuth](c, value)
}

func (c FfiConverterStaticJwtAuth) Write(writer io.Writer, value StaticJwtAuth) {
	FfiConverterStringINSTANCE.Write(writer, value.TokenFile)
	FfiConverterDurationINSTANCE.Write(writer, value.Duration)
}

type FfiDestroyerStaticJwtAuth struct{}

func (_ FfiDestroyerStaticJwtAuth) Destroy(value StaticJwtAuth) {
	value.Destroy()
}

// TLS configuration for client connections
type TlsClientConfig struct {
	// Disable TLS entirely (plain text connection)
	Insecure bool
	// Skip server certificate verification (enables TLS but doesn't verify certs)
	// WARNING: Only use for testing - insecure in production!
	InsecureSkipVerify bool
	// Certificate and key source for client authentication
	Source TlsSource
	// CA certificate source for verifying server certificates
	CaSource CaSource
	// Include system CA certificates pool (default: true)
	IncludeSystemCaCertsPool bool
	// TLS version to use: "tls1.2" or "tls1.3" (default: "tls1.3")
	TlsVersion string
}

func (r *TlsClientConfig) Destroy() {
	FfiDestroyerBool{}.Destroy(r.Insecure)
	FfiDestroyerBool{}.Destroy(r.InsecureSkipVerify)
	FfiDestroyerTlsSource{}.Destroy(r.Source)
	FfiDestroyerCaSource{}.Destroy(r.CaSource)
	FfiDestroyerBool{}.Destroy(r.IncludeSystemCaCertsPool)
	FfiDestroyerString{}.Destroy(r.TlsVersion)
}

type FfiConverterTlsClientConfig struct{}

var FfiConverterTlsClientConfigINSTANCE = FfiConverterTlsClientConfig{}

func (c FfiConverterTlsClientConfig) Lift(rb RustBufferI) TlsClientConfig {
	return LiftFromRustBuffer[TlsClientConfig](c, rb)
}

func (c FfiConverterTlsClientConfig) Read(reader io.Reader) TlsClientConfig {
	return TlsClientConfig{
		FfiConverterBoolINSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
		FfiConverterTlsSourceINSTANCE.Read(reader),
		FfiConverterCaSourceINSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
		FfiConverterStringINSTANCE.Read(reader),
	}
}

func (c FfiConverterTlsClientConfig) Lower(value TlsClientConfig) C.RustBuffer {
	return LowerIntoRustBuffer[TlsClientConfig](c, value)
}

func (c FfiConverterTlsClientConfig) Write(writer io.Writer, value TlsClientConfig) {
	FfiConverterBoolINSTANCE.Write(writer, value.Insecure)
	FfiConverterBoolINSTANCE.Write(writer, value.InsecureSkipVerify)
	FfiConverterTlsSourceINSTANCE.Write(writer, value.Source)
	FfiConverterCaSourceINSTANCE.Write(writer, value.CaSource)
	FfiConverterBoolINSTANCE.Write(writer, value.IncludeSystemCaCertsPool)
	FfiConverterStringINSTANCE.Write(writer, value.TlsVersion)
}

type FfiDestroyerTlsClientConfig struct{}

func (_ FfiDestroyerTlsClientConfig) Destroy(value TlsClientConfig) {
	value.Destroy()
}

// TLS configuration for server connections
type TlsServerConfig struct {
	// Disable TLS entirely (plain text connection)
	Insecure bool
	// Certificate and key source for server authentication
	Source TlsSource
	// CA certificate source for verifying client certificates
	ClientCa CaSource
	// Include system CA certificates pool (default: true)
	IncludeSystemCaCertsPool bool
	// TLS version to use: "tls1.2" or "tls1.3" (default: "tls1.3")
	TlsVersion string
	// Reload client CA file when modified
	ReloadClientCaFile bool
}

func (r *TlsServerConfig) Destroy() {
	FfiDestroyerBool{}.Destroy(r.Insecure)
	FfiDestroyerTlsSource{}.Destroy(r.Source)
	FfiDestroyerCaSource{}.Destroy(r.ClientCa)
	FfiDestroyerBool{}.Destroy(r.IncludeSystemCaCertsPool)
	FfiDestroyerString{}.Destroy(r.TlsVersion)
	FfiDestroyerBool{}.Destroy(r.ReloadClientCaFile)
}

type FfiConverterTlsServerConfig struct{}

var FfiConverterTlsServerConfigINSTANCE = FfiConverterTlsServerConfig{}

func (c FfiConverterTlsServerConfig) Lift(rb RustBufferI) TlsServerConfig {
	return LiftFromRustBuffer[TlsServerConfig](c, rb)
}

func (c FfiConverterTlsServerConfig) Read(reader io.Reader) TlsServerConfig {
	return TlsServerConfig{
		FfiConverterBoolINSTANCE.Read(reader),
		FfiConverterTlsSourceINSTANCE.Read(reader),
		FfiConverterCaSourceINSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
	}
}

func (c FfiConverterTlsServerConfig) Lower(value TlsServerConfig) C.RustBuffer {
	return LowerIntoRustBuffer[TlsServerConfig](c, value)
}

func (c FfiConverterTlsServerConfig) Write(writer io.Writer, value TlsServerConfig) {
	FfiConverterBoolINSTANCE.Write(writer, value.Insecure)
	FfiConverterTlsSourceINSTANCE.Write(writer, value.Source)
	FfiConverterCaSourceINSTANCE.Write(writer, value.ClientCa)
	FfiConverterBoolINSTANCE.Write(writer, value.IncludeSystemCaCertsPool)
	FfiConverterStringINSTANCE.Write(writer, value.TlsVersion)
	FfiConverterBoolINSTANCE.Write(writer, value.ReloadClientCaFile)
}

type FfiDestroyerTlsServerConfig struct{}

func (_ FfiDestroyerTlsServerConfig) Destroy(value TlsServerConfig) {
	value.Destroy()
}

// Backoff retry configuration
type BackoffConfig interface {
	Destroy()
}
type BackoffConfigExponential struct {
	Config ExponentialBackoff
}

func (e BackoffConfigExponential) Destroy() {
	FfiDestroyerExponentialBackoff{}.Destroy(e.Config)
}

type BackoffConfigFixedInterval struct {
	Config FixedIntervalBackoff
}

func (e BackoffConfigFixedInterval) Destroy() {
	FfiDestroyerFixedIntervalBackoff{}.Destroy(e.Config)
}

type FfiConverterBackoffConfig struct{}

var FfiConverterBackoffConfigINSTANCE = FfiConverterBackoffConfig{}

func (c FfiConverterBackoffConfig) Lift(rb RustBufferI) BackoffConfig {
	return LiftFromRustBuffer[BackoffConfig](c, rb)
}

func (c FfiConverterBackoffConfig) Lower(value BackoffConfig) C.RustBuffer {
	return LowerIntoRustBuffer[BackoffConfig](c, value)
}
func (FfiConverterBackoffConfig) Read(reader io.Reader) BackoffConfig {
	id := readInt32(reader)
	switch id {
	case 1:
		return BackoffConfigExponential{
			FfiConverterExponentialBackoffINSTANCE.Read(reader),
		}
	case 2:
		return BackoffConfigFixedInterval{
			FfiConverterFixedIntervalBackoffINSTANCE.Read(reader),
		}
	default:
		panic(fmt.Sprintf("invalid enum value %v in FfiConverterBackoffConfig.Read()", id))
	}
}

func (FfiConverterBackoffConfig) Write(writer io.Writer, value BackoffConfig) {
	switch variant_value := value.(type) {
	case BackoffConfigExponential:
		writeInt32(writer, 1)
		FfiConverterExponentialBackoffINSTANCE.Write(writer, variant_value.Config)
	case BackoffConfigFixedInterval:
		writeInt32(writer, 2)
		FfiConverterFixedIntervalBackoffINSTANCE.Write(writer, variant_value.Config)
	default:
		_ = variant_value
		panic(fmt.Sprintf("invalid enum value `%v` in FfiConverterBackoffConfig.Write", value))
	}
}

type FfiDestroyerBackoffConfig struct{}

func (_ FfiDestroyerBackoffConfig) Destroy(value BackoffConfig) {
	value.Destroy()
}

// CA certificate source configuration
type CaSource interface {
	Destroy()
}

// Load CA from file
type CaSourceFile struct {
	Path string
}

func (e CaSourceFile) Destroy() {
	FfiDestroyerString{}.Destroy(e.Path)
}

// Load CA from PEM string
type CaSourcePem struct {
	Data string
}

func (e CaSourcePem) Destroy() {
	FfiDestroyerString{}.Destroy(e.Data)
}

// Load CA from SPIRE Workload API
type CaSourceSpire struct {
	Config SpireConfig
}

func (e CaSourceSpire) Destroy() {
	FfiDestroyerSpireConfig{}.Destroy(e.Config)
}

// No CA configured
type CaSourceNone struct {
}

func (e CaSourceNone) Destroy() {
}

type FfiConverterCaSource struct{}

var FfiConverterCaSourceINSTANCE = FfiConverterCaSource{}

func (c FfiConverterCaSource) Lift(rb RustBufferI) CaSource {
	return LiftFromRustBuffer[CaSource](c, rb)
}

func (c FfiConverterCaSource) Lower(value CaSource) C.RustBuffer {
	return LowerIntoRustBuffer[CaSource](c, value)
}
func (FfiConverterCaSource) Read(reader io.Reader) CaSource {
	id := readInt32(reader)
	switch id {
	case 1:
		return CaSourceFile{
			FfiConverterStringINSTANCE.Read(reader),
		}
	case 2:
		return CaSourcePem{
			FfiConverterStringINSTANCE.Read(reader),
		}
	case 3:
		return CaSourceSpire{
			FfiConverterSpireConfigINSTANCE.Read(reader),
		}
	case 4:
		return CaSourceNone{}
	default:
		panic(fmt.Sprintf("invalid enum value %v in FfiConverterCaSource.Read()", id))
	}
}

func (FfiConverterCaSource) Write(writer io.Writer, value CaSource) {
	switch variant_value := value.(type) {
	case CaSourceFile:
		writeInt32(writer, 1)
		FfiConverterStringINSTANCE.Write(writer, variant_value.Path)
	case CaSourcePem:
		writeInt32(writer, 2)
		FfiConverterStringINSTANCE.Write(writer, variant_value.Data)
	case CaSourceSpire:
		writeInt32(writer, 3)
		FfiConverterSpireConfigINSTANCE.Write(writer, variant_value.Config)
	case CaSourceNone:
		writeInt32(writer, 4)
	default:
		_ = variant_value
		panic(fmt.Sprintf("invalid enum value `%v` in FfiConverterCaSource.Write", value))
	}
}

type FfiDestroyerCaSource struct{}

func (_ FfiDestroyerCaSource) Destroy(value CaSource) {
	value.Destroy()
}

// Authentication configuration enum for client
type ClientAuthenticationConfig interface {
	Destroy()
}
type ClientAuthenticationConfigBasic struct {
	Config BasicAuth
}

func (e ClientAuthenticationConfigBasic) Destroy() {
	FfiDestroyerBasicAuth{}.Destroy(e.Config)
}

type ClientAuthenticationConfigStaticJwt struct {
	Config StaticJwtAuth
}

func (e ClientAuthenticationConfigStaticJwt) Destroy() {
	FfiDestroyerStaticJwtAuth{}.Destroy(e.Config)
}

type ClientAuthenticationConfigJwt struct {
	Config ClientJwtAuth
}

func (e ClientAuthenticationConfigJwt) Destroy() {
	FfiDestroyerClientJwtAuth{}.Destroy(e.Config)
}

type ClientAuthenticationConfigNone struct {
}

func (e ClientAuthenticationConfigNone) Destroy() {
}

type FfiConverterClientAuthenticationConfig struct{}

var FfiConverterClientAuthenticationConfigINSTANCE = FfiConverterClientAuthenticationConfig{}

func (c FfiConverterClientAuthenticationConfig) Lift(rb RustBufferI) ClientAuthenticationConfig {
	return LiftFromRustBuffer[ClientAuthenticationConfig](c, rb)
}

func (c FfiConverterClientAuthenticationConfig) Lower(value ClientAuthenticationConfig) C.RustBuffer {
	return LowerIntoRustBuffer[ClientAuthenticationConfig](c, value)
}
func (FfiConverterClientAuthenticationConfig) Read(reader io.Reader) ClientAuthenticationConfig {
	id := readInt32(reader)
	switch id {
	case 1:
		return ClientAuthenticationConfigBasic{
			FfiConverterBasicAuthINSTANCE.Read(reader),
		}
	case 2:
		return ClientAuthenticationConfigStaticJwt{
			FfiConverterStaticJwtAuthINSTANCE.Read(reader),
		}
	case 3:
		return ClientAuthenticationConfigJwt{
			FfiConverterClientJwtAuthINSTANCE.Read(reader),
		}
	case 4:
		return ClientAuthenticationConfigNone{}
	default:
		panic(fmt.Sprintf("invalid enum value %v in FfiConverterClientAuthenticationConfig.Read()", id))
	}
}

func (FfiConverterClientAuthenticationConfig) Write(writer io.Writer, value ClientAuthenticationConfig) {
	switch variant_value := value.(type) {
	case ClientAuthenticationConfigBasic:
		writeInt32(writer, 1)
		FfiConverterBasicAuthINSTANCE.Write(writer, variant_value.Config)
	case ClientAuthenticationConfigStaticJwt:
		writeInt32(writer, 2)
		FfiConverterStaticJwtAuthINSTANCE.Write(writer, variant_value.Config)
	case ClientAuthenticationConfigJwt:
		writeInt32(writer, 3)
		FfiConverterClientJwtAuthINSTANCE.Write(writer, variant_value.Config)
	case ClientAuthenticationConfigNone:
		writeInt32(writer, 4)
	default:
		_ = variant_value
		panic(fmt.Sprintf("invalid enum value `%v` in FfiConverterClientAuthenticationConfig.Write", value))
	}
}

type FfiDestroyerClientAuthenticationConfig struct{}

func (_ FfiDestroyerClientAuthenticationConfig) Destroy(value ClientAuthenticationConfig) {
	value.Destroy()
}

// Compression type for gRPC messages
type CompressionType uint

const (
	CompressionTypeGzip    CompressionType = 1
	CompressionTypeZlib    CompressionType = 2
	CompressionTypeDeflate CompressionType = 3
	CompressionTypeSnappy  CompressionType = 4
	CompressionTypeZstd    CompressionType = 5
	CompressionTypeLz4     CompressionType = 6
	CompressionTypeNone    CompressionType = 7
	CompressionTypeEmpty   CompressionType = 8
)

type FfiConverterCompressionType struct{}

var FfiConverterCompressionTypeINSTANCE = FfiConverterCompressionType{}

func (c FfiConverterCompressionType) Lift(rb RustBufferI) CompressionType {
	return LiftFromRustBuffer[CompressionType](c, rb)
}

func (c FfiConverterCompressionType) Lower(value CompressionType) C.RustBuffer {
	return LowerIntoRustBuffer[CompressionType](c, value)
}
func (FfiConverterCompressionType) Read(reader io.Reader) CompressionType {
	id := readInt32(reader)
	return CompressionType(id)
}

func (FfiConverterCompressionType) Write(writer io.Writer, value CompressionType) {
	writeInt32(writer, int32(value))
}

type FfiDestroyerCompressionType struct{}

func (_ FfiDestroyerCompressionType) Destroy(value CompressionType) {
}

// JWT signing/verification algorithm
type JwtAlgorithm uint

const (
	JwtAlgorithmHs256 JwtAlgorithm = 1
	JwtAlgorithmHs384 JwtAlgorithm = 2
	JwtAlgorithmHs512 JwtAlgorithm = 3
	JwtAlgorithmEs256 JwtAlgorithm = 4
	JwtAlgorithmEs384 JwtAlgorithm = 5
	JwtAlgorithmRs256 JwtAlgorithm = 6
	JwtAlgorithmRs384 JwtAlgorithm = 7
	JwtAlgorithmRs512 JwtAlgorithm = 8
	JwtAlgorithmPs256 JwtAlgorithm = 9
	JwtAlgorithmPs384 JwtAlgorithm = 10
	JwtAlgorithmPs512 JwtAlgorithm = 11
	JwtAlgorithmEdDsa JwtAlgorithm = 12
)

type FfiConverterJwtAlgorithm struct{}

var FfiConverterJwtAlgorithmINSTANCE = FfiConverterJwtAlgorithm{}

func (c FfiConverterJwtAlgorithm) Lift(rb RustBufferI) JwtAlgorithm {
	return LiftFromRustBuffer[JwtAlgorithm](c, rb)
}

func (c FfiConverterJwtAlgorithm) Lower(value JwtAlgorithm) C.RustBuffer {
	return LowerIntoRustBuffer[JwtAlgorithm](c, value)
}
func (FfiConverterJwtAlgorithm) Read(reader io.Reader) JwtAlgorithm {
	id := readInt32(reader)
	return JwtAlgorithm(id)
}

func (FfiConverterJwtAlgorithm) Write(writer io.Writer, value JwtAlgorithm) {
	writeInt32(writer, int32(value))
}

type FfiDestroyerJwtAlgorithm struct{}

func (_ FfiDestroyerJwtAlgorithm) Destroy(value JwtAlgorithm) {
}

// JWT key data source
type JwtKeyData interface {
	Destroy()
}

// String with encoded key(s)
type JwtKeyDataData struct {
	Value string
}

func (e JwtKeyDataData) Destroy() {
	FfiDestroyerString{}.Destroy(e.Value)
}

// File path to the key(s)
type JwtKeyDataFile struct {
	Path string
}

func (e JwtKeyDataFile) Destroy() {
	FfiDestroyerString{}.Destroy(e.Path)
}

type FfiConverterJwtKeyData struct{}

var FfiConverterJwtKeyDataINSTANCE = FfiConverterJwtKeyData{}

func (c FfiConverterJwtKeyData) Lift(rb RustBufferI) JwtKeyData {
	return LiftFromRustBuffer[JwtKeyData](c, rb)
}

func (c FfiConverterJwtKeyData) Lower(value JwtKeyData) C.RustBuffer {
	return LowerIntoRustBuffer[JwtKeyData](c, value)
}
func (FfiConverterJwtKeyData) Read(reader io.Reader) JwtKeyData {
	id := readInt32(reader)
	switch id {
	case 1:
		return JwtKeyDataData{
			FfiConverterStringINSTANCE.Read(reader),
		}
	case 2:
		return JwtKeyDataFile{
			FfiConverterStringINSTANCE.Read(reader),
		}
	default:
		panic(fmt.Sprintf("invalid enum value %v in FfiConverterJwtKeyData.Read()", id))
	}
}

func (FfiConverterJwtKeyData) Write(writer io.Writer, value JwtKeyData) {
	switch variant_value := value.(type) {
	case JwtKeyDataData:
		writeInt32(writer, 1)
		FfiConverterStringINSTANCE.Write(writer, variant_value.Value)
	case JwtKeyDataFile:
		writeInt32(writer, 2)
		FfiConverterStringINSTANCE.Write(writer, variant_value.Path)
	default:
		_ = variant_value
		panic(fmt.Sprintf("invalid enum value `%v` in FfiConverterJwtKeyData.Write", value))
	}
}

type FfiDestroyerJwtKeyData struct{}

func (_ FfiDestroyerJwtKeyData) Destroy(value JwtKeyData) {
	value.Destroy()
}

// JWT key format
type JwtKeyFormat uint

const (
	JwtKeyFormatPem  JwtKeyFormat = 1
	JwtKeyFormatJwk  JwtKeyFormat = 2
	JwtKeyFormatJwks JwtKeyFormat = 3
)

type FfiConverterJwtKeyFormat struct{}

var FfiConverterJwtKeyFormatINSTANCE = FfiConverterJwtKeyFormat{}

func (c FfiConverterJwtKeyFormat) Lift(rb RustBufferI) JwtKeyFormat {
	return LiftFromRustBuffer[JwtKeyFormat](c, rb)
}

func (c FfiConverterJwtKeyFormat) Lower(value JwtKeyFormat) C.RustBuffer {
	return LowerIntoRustBuffer[JwtKeyFormat](c, value)
}
func (FfiConverterJwtKeyFormat) Read(reader io.Reader) JwtKeyFormat {
	id := readInt32(reader)
	return JwtKeyFormat(id)
}

func (FfiConverterJwtKeyFormat) Write(writer io.Writer, value JwtKeyFormat) {
	writeInt32(writer, int32(value))
}

type FfiDestroyerJwtKeyFormat struct{}

func (_ FfiDestroyerJwtKeyFormat) Destroy(value JwtKeyFormat) {
}

// JWT key type (encoding, decoding, or autoresolve)
type JwtKeyType interface {
	Destroy()
}

// Encoding key for signing JWTs (client-side)
type JwtKeyTypeEncoding struct {
	Key JwtKeyConfig
}

func (e JwtKeyTypeEncoding) Destroy() {
	FfiDestroyerJwtKeyConfig{}.Destroy(e.Key)
}

// Decoding key for verifying JWTs (server-side)
type JwtKeyTypeDecoding struct {
	Key JwtKeyConfig
}

func (e JwtKeyTypeDecoding) Destroy() {
	FfiDestroyerJwtKeyConfig{}.Destroy(e.Key)
}

// Automatically resolve keys based on claims
type JwtKeyTypeAutoresolve struct {
}

func (e JwtKeyTypeAutoresolve) Destroy() {
}

type FfiConverterJwtKeyType struct{}

var FfiConverterJwtKeyTypeINSTANCE = FfiConverterJwtKeyType{}

func (c FfiConverterJwtKeyType) Lift(rb RustBufferI) JwtKeyType {
	return LiftFromRustBuffer[JwtKeyType](c, rb)
}

func (c FfiConverterJwtKeyType) Lower(value JwtKeyType) C.RustBuffer {
	return LowerIntoRustBuffer[JwtKeyType](c, value)
}
func (FfiConverterJwtKeyType) Read(reader io.Reader) JwtKeyType {
	id := readInt32(reader)
	switch id {
	case 1:
		return JwtKeyTypeEncoding{
			FfiConverterJwtKeyConfigINSTANCE.Read(reader),
		}
	case 2:
		return JwtKeyTypeDecoding{
			FfiConverterJwtKeyConfigINSTANCE.Read(reader),
		}
	case 3:
		return JwtKeyTypeAutoresolve{}
	default:
		panic(fmt.Sprintf("invalid enum value %v in FfiConverterJwtKeyType.Read()", id))
	}
}

func (FfiConverterJwtKeyType) Write(writer io.Writer, value JwtKeyType) {
	switch variant_value := value.(type) {
	case JwtKeyTypeEncoding:
		writeInt32(writer, 1)
		FfiConverterJwtKeyConfigINSTANCE.Write(writer, variant_value.Key)
	case JwtKeyTypeDecoding:
		writeInt32(writer, 2)
		FfiConverterJwtKeyConfigINSTANCE.Write(writer, variant_value.Key)
	case JwtKeyTypeAutoresolve:
		writeInt32(writer, 3)
	default:
		_ = variant_value
		panic(fmt.Sprintf("invalid enum value `%v` in FfiConverterJwtKeyType.Write", value))
	}
}

type FfiDestroyerJwtKeyType struct{}

func (_ FfiDestroyerJwtKeyType) Destroy(value JwtKeyType) {
	value.Destroy()
}

// Authentication configuration enum for server
type ServerAuthenticationConfig interface {
	Destroy()
}
type ServerAuthenticationConfigBasic struct {
	Config BasicAuth
}

func (e ServerAuthenticationConfigBasic) Destroy() {
	FfiDestroyerBasicAuth{}.Destroy(e.Config)
}

type ServerAuthenticationConfigJwt struct {
	Config JwtAuth
}

func (e ServerAuthenticationConfigJwt) Destroy() {
	FfiDestroyerJwtAuth{}.Destroy(e.Config)
}

type ServerAuthenticationConfigNone struct {
}

func (e ServerAuthenticationConfigNone) Destroy() {
}

type FfiConverterServerAuthenticationConfig struct{}

var FfiConverterServerAuthenticationConfigINSTANCE = FfiConverterServerAuthenticationConfig{}

func (c FfiConverterServerAuthenticationConfig) Lift(rb RustBufferI) ServerAuthenticationConfig {
	return LiftFromRustBuffer[ServerAuthenticationConfig](c, rb)
}

func (c FfiConverterServerAuthenticationConfig) Lower(value ServerAuthenticationConfig) C.RustBuffer {
	return LowerIntoRustBuffer[ServerAuthenticationConfig](c, value)
}
func (FfiConverterServerAuthenticationConfig) Read(reader io.Reader) ServerAuthenticationConfig {
	id := readInt32(reader)
	switch id {
	case 1:
		return ServerAuthenticationConfigBasic{
			FfiConverterBasicAuthINSTANCE.Read(reader),
		}
	case 2:
		return ServerAuthenticationConfigJwt{
			FfiConverterJwtAuthINSTANCE.Read(reader),
		}
	case 3:
		return ServerAuthenticationConfigNone{}
	default:
		panic(fmt.Sprintf("invalid enum value %v in FfiConverterServerAuthenticationConfig.Read()", id))
	}
}

func (FfiConverterServerAuthenticationConfig) Write(writer io.Writer, value ServerAuthenticationConfig) {
	switch variant_value := value.(type) {
	case ServerAuthenticationConfigBasic:
		writeInt32(writer, 1)
		FfiConverterBasicAuthINSTANCE.Write(writer, variant_value.Config)
	case ServerAuthenticationConfigJwt:
		writeInt32(writer, 2)
		FfiConverterJwtAuthINSTANCE.Write(writer, variant_value.Config)
	case ServerAuthenticationConfigNone:
		writeInt32(writer, 3)
	default:
		_ = variant_value
		panic(fmt.Sprintf("invalid enum value `%v` in FfiConverterServerAuthenticationConfig.Write", value))
	}
}

type FfiDestroyerServerAuthenticationConfig struct{}

func (_ FfiDestroyerServerAuthenticationConfig) Destroy(value ServerAuthenticationConfig) {
	value.Destroy()
}

// Session type enum
type SessionType uint

const (
	SessionTypePointToPoint SessionType = 1
	SessionTypeGroup        SessionType = 2
)

type FfiConverterSessionType struct{}

var FfiConverterSessionTypeINSTANCE = FfiConverterSessionType{}

func (c FfiConverterSessionType) Lift(rb RustBufferI) SessionType {
	return LiftFromRustBuffer[SessionType](c, rb)
}

func (c FfiConverterSessionType) Lower(value SessionType) C.RustBuffer {
	return LowerIntoRustBuffer[SessionType](c, value)
}
func (FfiConverterSessionType) Read(reader io.Reader) SessionType {
	id := readInt32(reader)
	return SessionType(id)
}

func (FfiConverterSessionType) Write(writer io.Writer, value SessionType) {
	writeInt32(writer, int32(value))
}

type FfiDestroyerSessionType struct{}

func (_ FfiDestroyerSessionType) Destroy(value SessionType) {
}

// Error types for SLIM operations
type SlimError struct {
	err error
}

// Convience method to turn *SlimError into error
// Avoiding treating nil pointer as non nil error interface
func (err *SlimError) AsError() error {
	if err == nil {
		return nil
	} else {
		return err
	}
}

func (err SlimError) Error() string {
	return fmt.Sprintf("SlimError: %s", err.err.Error())
}

func (err SlimError) Unwrap() error {
	return err.err
}

// Err* are used for checking error type with `errors.Is`
var ErrSlimErrorServiceError = fmt.Errorf("SlimErrorServiceError")
var ErrSlimErrorSessionError = fmt.Errorf("SlimErrorSessionError")
var ErrSlimErrorReceiveError = fmt.Errorf("SlimErrorReceiveError")
var ErrSlimErrorSendError = fmt.Errorf("SlimErrorSendError")
var ErrSlimErrorAuthError = fmt.Errorf("SlimErrorAuthError")
var ErrSlimErrorTimeout = fmt.Errorf("SlimErrorTimeout")
var ErrSlimErrorInvalidArgument = fmt.Errorf("SlimErrorInvalidArgument")
var ErrSlimErrorInternalError = fmt.Errorf("SlimErrorInternalError")

// Variant structs
type SlimErrorServiceError struct {
	Message string
}

func NewSlimErrorServiceError(
	message string,
) *SlimError {
	return &SlimError{err: &SlimErrorServiceError{
		Message: message}}
}

func (e SlimErrorServiceError) destroy() {
	FfiDestroyerString{}.Destroy(e.Message)
}

func (err SlimErrorServiceError) Error() string {
	return fmt.Sprint("ServiceError",
		": ",

		"Message=",
		err.Message,
	)
}

func (self SlimErrorServiceError) Is(target error) bool {
	return target == ErrSlimErrorServiceError
}

type SlimErrorSessionError struct {
	Message string
}

func NewSlimErrorSessionError(
	message string,
) *SlimError {
	return &SlimError{err: &SlimErrorSessionError{
		Message: message}}
}

func (e SlimErrorSessionError) destroy() {
	FfiDestroyerString{}.Destroy(e.Message)
}

func (err SlimErrorSessionError) Error() string {
	return fmt.Sprint("SessionError",
		": ",

		"Message=",
		err.Message,
	)
}

func (self SlimErrorSessionError) Is(target error) bool {
	return target == ErrSlimErrorSessionError
}

type SlimErrorReceiveError struct {
	Message string
}

func NewSlimErrorReceiveError(
	message string,
) *SlimError {
	return &SlimError{err: &SlimErrorReceiveError{
		Message: message}}
}

func (e SlimErrorReceiveError) destroy() {
	FfiDestroyerString{}.Destroy(e.Message)
}

func (err SlimErrorReceiveError) Error() string {
	return fmt.Sprint("ReceiveError",
		": ",

		"Message=",
		err.Message,
	)
}

func (self SlimErrorReceiveError) Is(target error) bool {
	return target == ErrSlimErrorReceiveError
}

type SlimErrorSendError struct {
	Message string
}

func NewSlimErrorSendError(
	message string,
) *SlimError {
	return &SlimError{err: &SlimErrorSendError{
		Message: message}}
}

func (e SlimErrorSendError) destroy() {
	FfiDestroyerString{}.Destroy(e.Message)
}

func (err SlimErrorSendError) Error() string {
	return fmt.Sprint("SendError",
		": ",

		"Message=",
		err.Message,
	)
}

func (self SlimErrorSendError) Is(target error) bool {
	return target == ErrSlimErrorSendError
}

type SlimErrorAuthError struct {
	Message string
}

func NewSlimErrorAuthError(
	message string,
) *SlimError {
	return &SlimError{err: &SlimErrorAuthError{
		Message: message}}
}

func (e SlimErrorAuthError) destroy() {
	FfiDestroyerString{}.Destroy(e.Message)
}

func (err SlimErrorAuthError) Error() string {
	return fmt.Sprint("AuthError",
		": ",

		"Message=",
		err.Message,
	)
}

func (self SlimErrorAuthError) Is(target error) bool {
	return target == ErrSlimErrorAuthError
}

type SlimErrorTimeout struct {
}

func NewSlimErrorTimeout() *SlimError {
	return &SlimError{err: &SlimErrorTimeout{}}
}

func (e SlimErrorTimeout) destroy() {
}

func (err SlimErrorTimeout) Error() string {
	return fmt.Sprint("Timeout")
}

func (self SlimErrorTimeout) Is(target error) bool {
	return target == ErrSlimErrorTimeout
}

type SlimErrorInvalidArgument struct {
	Message string
}

func NewSlimErrorInvalidArgument(
	message string,
) *SlimError {
	return &SlimError{err: &SlimErrorInvalidArgument{
		Message: message}}
}

func (e SlimErrorInvalidArgument) destroy() {
	FfiDestroyerString{}.Destroy(e.Message)
}

func (err SlimErrorInvalidArgument) Error() string {
	return fmt.Sprint("InvalidArgument",
		": ",

		"Message=",
		err.Message,
	)
}

func (self SlimErrorInvalidArgument) Is(target error) bool {
	return target == ErrSlimErrorInvalidArgument
}

type SlimErrorInternalError struct {
	Message string
}

func NewSlimErrorInternalError(
	message string,
) *SlimError {
	return &SlimError{err: &SlimErrorInternalError{
		Message: message}}
}

func (e SlimErrorInternalError) destroy() {
	FfiDestroyerString{}.Destroy(e.Message)
}

func (err SlimErrorInternalError) Error() string {
	return fmt.Sprint("InternalError",
		": ",

		"Message=",
		err.Message,
	)
}

func (self SlimErrorInternalError) Is(target error) bool {
	return target == ErrSlimErrorInternalError
}

type FfiConverterSlimError struct{}

var FfiConverterSlimErrorINSTANCE = FfiConverterSlimError{}

func (c FfiConverterSlimError) Lift(eb RustBufferI) *SlimError {
	return LiftFromRustBuffer[*SlimError](c, eb)
}

func (c FfiConverterSlimError) Lower(value *SlimError) C.RustBuffer {
	return LowerIntoRustBuffer[*SlimError](c, value)
}

func (c FfiConverterSlimError) Read(reader io.Reader) *SlimError {
	errorID := readUint32(reader)

	switch errorID {
	case 1:
		return &SlimError{&SlimErrorServiceError{
			Message: FfiConverterStringINSTANCE.Read(reader),
		}}
	case 2:
		return &SlimError{&SlimErrorSessionError{
			Message: FfiConverterStringINSTANCE.Read(reader),
		}}
	case 3:
		return &SlimError{&SlimErrorReceiveError{
			Message: FfiConverterStringINSTANCE.Read(reader),
		}}
	case 4:
		return &SlimError{&SlimErrorSendError{
			Message: FfiConverterStringINSTANCE.Read(reader),
		}}
	case 5:
		return &SlimError{&SlimErrorAuthError{
			Message: FfiConverterStringINSTANCE.Read(reader),
		}}
	case 6:
		return &SlimError{&SlimErrorTimeout{}}
	case 7:
		return &SlimError{&SlimErrorInvalidArgument{
			Message: FfiConverterStringINSTANCE.Read(reader),
		}}
	case 8:
		return &SlimError{&SlimErrorInternalError{
			Message: FfiConverterStringINSTANCE.Read(reader),
		}}
	default:
		panic(fmt.Sprintf("Unknown error code %d in FfiConverterSlimError.Read()", errorID))
	}
}

func (c FfiConverterSlimError) Write(writer io.Writer, value *SlimError) {
	switch variantValue := value.err.(type) {
	case *SlimErrorServiceError:
		writeInt32(writer, 1)
		FfiConverterStringINSTANCE.Write(writer, variantValue.Message)
	case *SlimErrorSessionError:
		writeInt32(writer, 2)
		FfiConverterStringINSTANCE.Write(writer, variantValue.Message)
	case *SlimErrorReceiveError:
		writeInt32(writer, 3)
		FfiConverterStringINSTANCE.Write(writer, variantValue.Message)
	case *SlimErrorSendError:
		writeInt32(writer, 4)
		FfiConverterStringINSTANCE.Write(writer, variantValue.Message)
	case *SlimErrorAuthError:
		writeInt32(writer, 5)
		FfiConverterStringINSTANCE.Write(writer, variantValue.Message)
	case *SlimErrorTimeout:
		writeInt32(writer, 6)
	case *SlimErrorInvalidArgument:
		writeInt32(writer, 7)
		FfiConverterStringINSTANCE.Write(writer, variantValue.Message)
	case *SlimErrorInternalError:
		writeInt32(writer, 8)
		FfiConverterStringINSTANCE.Write(writer, variantValue.Message)
	default:
		_ = variantValue
		panic(fmt.Sprintf("invalid error value `%v` in FfiConverterSlimError.Write", value))
	}
}

type FfiDestroyerSlimError struct{}

func (_ FfiDestroyerSlimError) Destroy(value *SlimError) {
	switch variantValue := value.err.(type) {
	case SlimErrorServiceError:
		variantValue.destroy()
	case SlimErrorSessionError:
		variantValue.destroy()
	case SlimErrorReceiveError:
		variantValue.destroy()
	case SlimErrorSendError:
		variantValue.destroy()
	case SlimErrorAuthError:
		variantValue.destroy()
	case SlimErrorTimeout:
		variantValue.destroy()
	case SlimErrorInvalidArgument:
		variantValue.destroy()
	case SlimErrorInternalError:
		variantValue.destroy()
	default:
		_ = variantValue
		panic(fmt.Sprintf("invalid error value `%v` in FfiDestroyerSlimError.Destroy", value))
	}
}

// TLS certificate and key source configuration
type TlsSource interface {
	Destroy()
}

// Load certificate and key from PEM strings
type TlsSourcePem struct {
	Cert string
	Key  string
}

func (e TlsSourcePem) Destroy() {
	FfiDestroyerString{}.Destroy(e.Cert)
	FfiDestroyerString{}.Destroy(e.Key)
}

// Load certificate and key from files (with auto-reload support)
type TlsSourceFile struct {
	Cert string
	Key  string
}

func (e TlsSourceFile) Destroy() {
	FfiDestroyerString{}.Destroy(e.Cert)
	FfiDestroyerString{}.Destroy(e.Key)
}

// Load certificate and key from SPIRE Workload API
type TlsSourceSpire struct {
	Config SpireConfig
}

func (e TlsSourceSpire) Destroy() {
	FfiDestroyerSpireConfig{}.Destroy(e.Config)
}

// No certificate/key configured
type TlsSourceNone struct {
}

func (e TlsSourceNone) Destroy() {
}

type FfiConverterTlsSource struct{}

var FfiConverterTlsSourceINSTANCE = FfiConverterTlsSource{}

func (c FfiConverterTlsSource) Lift(rb RustBufferI) TlsSource {
	return LiftFromRustBuffer[TlsSource](c, rb)
}

func (c FfiConverterTlsSource) Lower(value TlsSource) C.RustBuffer {
	return LowerIntoRustBuffer[TlsSource](c, value)
}
func (FfiConverterTlsSource) Read(reader io.Reader) TlsSource {
	id := readInt32(reader)
	switch id {
	case 1:
		return TlsSourcePem{
			FfiConverterStringINSTANCE.Read(reader),
			FfiConverterStringINSTANCE.Read(reader),
		}
	case 2:
		return TlsSourceFile{
			FfiConverterStringINSTANCE.Read(reader),
			FfiConverterStringINSTANCE.Read(reader),
		}
	case 3:
		return TlsSourceSpire{
			FfiConverterSpireConfigINSTANCE.Read(reader),
		}
	case 4:
		return TlsSourceNone{}
	default:
		panic(fmt.Sprintf("invalid enum value %v in FfiConverterTlsSource.Read()", id))
	}
}

func (FfiConverterTlsSource) Write(writer io.Writer, value TlsSource) {
	switch variant_value := value.(type) {
	case TlsSourcePem:
		writeInt32(writer, 1)
		FfiConverterStringINSTANCE.Write(writer, variant_value.Cert)
		FfiConverterStringINSTANCE.Write(writer, variant_value.Key)
	case TlsSourceFile:
		writeInt32(writer, 2)
		FfiConverterStringINSTANCE.Write(writer, variant_value.Cert)
		FfiConverterStringINSTANCE.Write(writer, variant_value.Key)
	case TlsSourceSpire:
		writeInt32(writer, 3)
		FfiConverterSpireConfigINSTANCE.Write(writer, variant_value.Config)
	case TlsSourceNone:
		writeInt32(writer, 4)
	default:
		_ = variant_value
		panic(fmt.Sprintf("invalid enum value `%v` in FfiConverterTlsSource.Write", value))
	}
}

type FfiDestroyerTlsSource struct{}

func (_ FfiDestroyerTlsSource) Destroy(value TlsSource) {
	value.Destroy()
}

type FfiConverterOptionalUint32 struct{}

var FfiConverterOptionalUint32INSTANCE = FfiConverterOptionalUint32{}

func (c FfiConverterOptionalUint32) Lift(rb RustBufferI) *uint32 {
	return LiftFromRustBuffer[*uint32](c, rb)
}

func (_ FfiConverterOptionalUint32) Read(reader io.Reader) *uint32 {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterUint32INSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalUint32) Lower(value *uint32) C.RustBuffer {
	return LowerIntoRustBuffer[*uint32](c, value)
}

func (_ FfiConverterOptionalUint32) Write(writer io.Writer, value *uint32) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterUint32INSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalUint32 struct{}

func (_ FfiDestroyerOptionalUint32) Destroy(value *uint32) {
	if value != nil {
		FfiDestroyerUint32{}.Destroy(*value)
	}
}

type FfiConverterOptionalUint64 struct{}

var FfiConverterOptionalUint64INSTANCE = FfiConverterOptionalUint64{}

func (c FfiConverterOptionalUint64) Lift(rb RustBufferI) *uint64 {
	return LiftFromRustBuffer[*uint64](c, rb)
}

func (_ FfiConverterOptionalUint64) Read(reader io.Reader) *uint64 {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterUint64INSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalUint64) Lower(value *uint64) C.RustBuffer {
	return LowerIntoRustBuffer[*uint64](c, value)
}

func (_ FfiConverterOptionalUint64) Write(writer io.Writer, value *uint64) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterUint64INSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalUint64 struct{}

func (_ FfiDestroyerOptionalUint64) Destroy(value *uint64) {
	if value != nil {
		FfiDestroyerUint64{}.Destroy(*value)
	}
}

type FfiConverterOptionalString struct{}

var FfiConverterOptionalStringINSTANCE = FfiConverterOptionalString{}

func (c FfiConverterOptionalString) Lift(rb RustBufferI) *string {
	return LiftFromRustBuffer[*string](c, rb)
}

func (_ FfiConverterOptionalString) Read(reader io.Reader) *string {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterStringINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalString) Lower(value *string) C.RustBuffer {
	return LowerIntoRustBuffer[*string](c, value)
}

func (_ FfiConverterOptionalString) Write(writer io.Writer, value *string) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterStringINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalString struct{}

func (_ FfiDestroyerOptionalString) Destroy(value *string) {
	if value != nil {
		FfiDestroyerString{}.Destroy(*value)
	}
}

type FfiConverterOptionalDuration struct{}

var FfiConverterOptionalDurationINSTANCE = FfiConverterOptionalDuration{}

func (c FfiConverterOptionalDuration) Lift(rb RustBufferI) *time.Duration {
	return LiftFromRustBuffer[*time.Duration](c, rb)
}

func (_ FfiConverterOptionalDuration) Read(reader io.Reader) *time.Duration {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterDurationINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalDuration) Lower(value *time.Duration) C.RustBuffer {
	return LowerIntoRustBuffer[*time.Duration](c, value)
}

func (_ FfiConverterOptionalDuration) Write(writer io.Writer, value *time.Duration) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterDurationINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalDuration struct{}

func (_ FfiDestroyerOptionalDuration) Destroy(value *time.Duration) {
	if value != nil {
		FfiDestroyerDuration{}.Destroy(*value)
	}
}

type FfiConverterOptionalName struct{}

var FfiConverterOptionalNameINSTANCE = FfiConverterOptionalName{}

func (c FfiConverterOptionalName) Lift(rb RustBufferI) **Name {
	return LiftFromRustBuffer[**Name](c, rb)
}

func (_ FfiConverterOptionalName) Read(reader io.Reader) **Name {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterNameINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalName) Lower(value **Name) C.RustBuffer {
	return LowerIntoRustBuffer[**Name](c, value)
}

func (_ FfiConverterOptionalName) Write(writer io.Writer, value **Name) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterNameINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalName struct{}

func (_ FfiDestroyerOptionalName) Destroy(value **Name) {
	if value != nil {
		FfiDestroyerName{}.Destroy(*value)
	}
}

type FfiConverterOptionalKeepaliveConfig struct{}

var FfiConverterOptionalKeepaliveConfigINSTANCE = FfiConverterOptionalKeepaliveConfig{}

func (c FfiConverterOptionalKeepaliveConfig) Lift(rb RustBufferI) *KeepaliveConfig {
	return LiftFromRustBuffer[*KeepaliveConfig](c, rb)
}

func (_ FfiConverterOptionalKeepaliveConfig) Read(reader io.Reader) *KeepaliveConfig {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterKeepaliveConfigINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalKeepaliveConfig) Lower(value *KeepaliveConfig) C.RustBuffer {
	return LowerIntoRustBuffer[*KeepaliveConfig](c, value)
}

func (_ FfiConverterOptionalKeepaliveConfig) Write(writer io.Writer, value *KeepaliveConfig) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterKeepaliveConfigINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalKeepaliveConfig struct{}

func (_ FfiDestroyerOptionalKeepaliveConfig) Destroy(value *KeepaliveConfig) {
	if value != nil {
		FfiDestroyerKeepaliveConfig{}.Destroy(*value)
	}
}

type FfiConverterOptionalCompressionType struct{}

var FfiConverterOptionalCompressionTypeINSTANCE = FfiConverterOptionalCompressionType{}

func (c FfiConverterOptionalCompressionType) Lift(rb RustBufferI) *CompressionType {
	return LiftFromRustBuffer[*CompressionType](c, rb)
}

func (_ FfiConverterOptionalCompressionType) Read(reader io.Reader) *CompressionType {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterCompressionTypeINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalCompressionType) Lower(value *CompressionType) C.RustBuffer {
	return LowerIntoRustBuffer[*CompressionType](c, value)
}

func (_ FfiConverterOptionalCompressionType) Write(writer io.Writer, value *CompressionType) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterCompressionTypeINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalCompressionType struct{}

func (_ FfiDestroyerOptionalCompressionType) Destroy(value *CompressionType) {
	if value != nil {
		FfiDestroyerCompressionType{}.Destroy(*value)
	}
}

type FfiConverterOptionalSequenceString struct{}

var FfiConverterOptionalSequenceStringINSTANCE = FfiConverterOptionalSequenceString{}

func (c FfiConverterOptionalSequenceString) Lift(rb RustBufferI) *[]string {
	return LiftFromRustBuffer[*[]string](c, rb)
}

func (_ FfiConverterOptionalSequenceString) Read(reader io.Reader) *[]string {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterSequenceStringINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalSequenceString) Lower(value *[]string) C.RustBuffer {
	return LowerIntoRustBuffer[*[]string](c, value)
}

func (_ FfiConverterOptionalSequenceString) Write(writer io.Writer, value *[]string) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterSequenceStringINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalSequenceString struct{}

func (_ FfiDestroyerOptionalSequenceString) Destroy(value *[]string) {
	if value != nil {
		FfiDestroyerSequenceString{}.Destroy(*value)
	}
}

type FfiConverterOptionalMapStringString struct{}

var FfiConverterOptionalMapStringStringINSTANCE = FfiConverterOptionalMapStringString{}

func (c FfiConverterOptionalMapStringString) Lift(rb RustBufferI) *map[string]string {
	return LiftFromRustBuffer[*map[string]string](c, rb)
}

func (_ FfiConverterOptionalMapStringString) Read(reader io.Reader) *map[string]string {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterMapStringStringINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalMapStringString) Lower(value *map[string]string) C.RustBuffer {
	return LowerIntoRustBuffer[*map[string]string](c, value)
}

func (_ FfiConverterOptionalMapStringString) Write(writer io.Writer, value *map[string]string) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterMapStringStringINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalMapStringString struct{}

func (_ FfiDestroyerOptionalMapStringString) Destroy(value *map[string]string) {
	if value != nil {
		FfiDestroyerMapStringString{}.Destroy(*value)
	}
}

type FfiConverterSequenceString struct{}

var FfiConverterSequenceStringINSTANCE = FfiConverterSequenceString{}

func (c FfiConverterSequenceString) Lift(rb RustBufferI) []string {
	return LiftFromRustBuffer[[]string](c, rb)
}

func (c FfiConverterSequenceString) Read(reader io.Reader) []string {
	length := readInt32(reader)
	if length == 0 {
		return nil
	}
	result := make([]string, 0, length)
	for i := int32(0); i < length; i++ {
		result = append(result, FfiConverterStringINSTANCE.Read(reader))
	}
	return result
}

func (c FfiConverterSequenceString) Lower(value []string) C.RustBuffer {
	return LowerIntoRustBuffer[[]string](c, value)
}

func (c FfiConverterSequenceString) Write(writer io.Writer, value []string) {
	if len(value) > math.MaxInt32 {
		panic("[]string is too large to fit into Int32")
	}

	writeInt32(writer, int32(len(value)))
	for _, item := range value {
		FfiConverterStringINSTANCE.Write(writer, item)
	}
}

type FfiDestroyerSequenceString struct{}

func (FfiDestroyerSequenceString) Destroy(sequence []string) {
	for _, value := range sequence {
		FfiDestroyerString{}.Destroy(value)
	}
}

type FfiConverterMapStringString struct{}

var FfiConverterMapStringStringINSTANCE = FfiConverterMapStringString{}

func (c FfiConverterMapStringString) Lift(rb RustBufferI) map[string]string {
	return LiftFromRustBuffer[map[string]string](c, rb)
}

func (_ FfiConverterMapStringString) Read(reader io.Reader) map[string]string {
	result := make(map[string]string)
	length := readInt32(reader)
	for i := int32(0); i < length; i++ {
		key := FfiConverterStringINSTANCE.Read(reader)
		value := FfiConverterStringINSTANCE.Read(reader)
		result[key] = value
	}
	return result
}

func (c FfiConverterMapStringString) Lower(value map[string]string) C.RustBuffer {
	return LowerIntoRustBuffer[map[string]string](c, value)
}

func (_ FfiConverterMapStringString) Write(writer io.Writer, mapValue map[string]string) {
	if len(mapValue) > math.MaxInt32 {
		panic("map[string]string is too large to fit into Int32")
	}

	writeInt32(writer, int32(len(mapValue)))
	for key, value := range mapValue {
		FfiConverterStringINSTANCE.Write(writer, key)
		FfiConverterStringINSTANCE.Write(writer, value)
	}
}

type FfiDestroyerMapStringString struct{}

func (_ FfiDestroyerMapStringString) Destroy(mapValue map[string]string) {
	for key, value := range mapValue {
		FfiDestroyerString{}.Destroy(key)
		FfiDestroyerString{}.Destroy(value)
	}
}

const (
	uniffiRustFuturePollReady      int8 = 0
	uniffiRustFuturePollMaybeReady int8 = 1
)

type rustFuturePollFunc func(C.uint64_t, C.UniffiRustFutureContinuationCallback, C.uint64_t)
type rustFutureCompleteFunc[T any] func(C.uint64_t, *C.RustCallStatus) T
type rustFutureFreeFunc func(C.uint64_t)

//export slim_bindings_uniffiFutureContinuationCallback
func slim_bindings_uniffiFutureContinuationCallback(data C.uint64_t, pollResult C.int8_t) {
	h := cgo.Handle(uintptr(data))
	waiter := h.Value().(chan int8)
	waiter <- int8(pollResult)
}

func uniffiRustCallAsync[E any, T any, F any](
	errConverter BufReader[*E],
	completeFunc rustFutureCompleteFunc[F],
	liftFunc func(F) T,
	rustFuture C.uint64_t,
	pollFunc rustFuturePollFunc,
	freeFunc rustFutureFreeFunc,
) (T, *E) {
	defer freeFunc(rustFuture)

	pollResult := int8(-1)
	waiter := make(chan int8, 1)

	chanHandle := cgo.NewHandle(waiter)
	defer chanHandle.Delete()

	for pollResult != uniffiRustFuturePollReady {
		pollFunc(
			rustFuture,
			(C.UniffiRustFutureContinuationCallback)(C.slim_bindings_uniffiFutureContinuationCallback),
			C.uint64_t(chanHandle),
		)
		pollResult = <-waiter
	}

	var goValue T
	var ffiValue F
	var err *E

	ffiValue, err = rustCallWithError(errConverter, func(status *C.RustCallStatus) F {
		return completeFunc(rustFuture, status)
	})
	if err != nil {
		return goValue, err
	}
	return liftFunc(ffiValue), nil
}

//export slim_bindings_uniffiFreeGorutine
func slim_bindings_uniffiFreeGorutine(data C.uint64_t) {
	handle := cgo.Handle(uintptr(data))
	defer handle.Delete()

	guard := handle.Value().(chan struct{})
	guard <- struct{}{}
}

// Create an app with the given name and shared secret (blocking version for FFI)
//
// This is the main entry point for creating a SLIM application from language bindings.
func CreateAppWithSecret(appName *Name, sharedSecret string) (*BindingsAdapter, error) {
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_func_create_app_with_secret(FfiConverterNameINSTANCE.Lower(appName), FfiConverterStringINSTANCE.Lower(sharedSecret), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *BindingsAdapter
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterBindingsAdapterINSTANCE.Lift(_uniffiRV), nil
	}
}

// Get detailed build information
func GetBuildInfo() BuildInfo {
	return FfiConverterBuildInfoINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_bindings_fn_func_get_build_info(_uniffiStatus),
		}
	}))
}

// Get the version of the SLIM bindings (simple string)
func GetVersion() string {
	return FfiConverterStringINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_bindings_fn_func_get_version(_uniffiStatus),
		}
	}))
}

// Initialize the crypto provider
//
// This must be called before any TLS operations. It's safe to call multiple times.
func InitializeCryptoProvider() {
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_func_initialize_crypto_provider(_uniffiStatus)
		return false
	})
}

// Create a new insecure client config (no TLS)
func NewInsecureClientConfig(endpoint string) ClientConfig {
	return FfiConverterClientConfigINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_bindings_fn_func_new_insecure_client_config(FfiConverterStringINSTANCE.Lower(endpoint), _uniffiStatus),
		}
	}))
}

// Create a new insecure server config (no TLS)
func NewInsecureServerConfig(endpoint string) ServerConfig {
	return FfiConverterServerConfigINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_bindings_fn_func_new_insecure_server_config(FfiConverterStringINSTANCE.Lower(endpoint), _uniffiStatus),
		}
	}))
}

// Create a new server config with the given endpoint and default values
func NewServerConfig(endpoint string) ServerConfig {
	return FfiConverterServerConfigINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_bindings_fn_func_new_server_config(FfiConverterStringINSTANCE.Lower(endpoint), _uniffiStatus),
		}
	}))
}
