package slim_bindings

/*
#cgo CFLAGS: -I${SRCDIR}
#cgo linux,amd64 LDFLAGS: -Wl,--start-group ${SRCDIR}/libslim_bindings_x86_64_linux_gnu_part0.a ${SRCDIR}/libslim_bindings_x86_64_linux_gnu_part1.a ${SRCDIR}/libslim_bindings_x86_64_linux_gnu_part2.a ${SRCDIR}/libslim_bindings_x86_64_linux_gnu_part3.a -Wl,--end-group -lm
#cgo linux,arm64 LDFLAGS: -Wl,--start-group ${SRCDIR}/libslim_bindings_aarch64_linux_gnu_part0.a ${SRCDIR}/libslim_bindings_aarch64_linux_gnu_part1.a ${SRCDIR}/libslim_bindings_aarch64_linux_gnu_part2.a ${SRCDIR}/libslim_bindings_aarch64_linux_gnu_part3.a -Wl,--end-group -lm
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
		if checksum != 8090 {
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
		if checksum != 51245 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_create_session: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_create_session_async()
		})
		if checksum != 41646 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_create_session_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_delete_session()
		})
		if checksum != 43581 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_delete_session: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_delete_session_async()
		})
		if checksum != 63854 {
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
		if checksum != 17892 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_listen_for_session: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_listen_for_session_async()
		})
		if checksum != 38361 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_listen_for_session_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_name()
		})
		if checksum != 52552 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_name: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_remove_route()
		})
		if checksum != 38193 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_remove_route: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_remove_route_async()
		})
		if checksum != 19850 {
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
		if checksum != 15592 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_set_route: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_set_route_async()
		})
		if checksum != 3907 {
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
		if checksum != 20649 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_subscribe: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_subscribe_async()
		})
		if checksum != 53497 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_subscribe_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_unsubscribe()
		})
		if checksum != 62896 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_unsubscribe: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingsadapter_unsubscribe_async()
		})
		if checksum != 10891 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingsadapter_unsubscribe_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_destination()
		})
		if checksum != 53278 {
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
		if checksum != 32120 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_invite: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_invite_async()
		})
		if checksum != 55626 {
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
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish()
		})
		if checksum != 63851 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_async()
		})
		if checksum != 36943 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to()
		})
		if checksum != 64932 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to_async()
		})
		if checksum != 46041 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to_with_completion()
		})
		if checksum != 5189 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to_with_completion: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to_with_completion_async()
		})
		if checksum != 44848 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_to_with_completion_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_with_completion()
		})
		if checksum != 36035 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_with_completion: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_with_completion_async()
		})
		if checksum != 29252 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_with_completion_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_with_params()
		})
		if checksum != 5665 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_with_params: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_with_params_async()
		})
		if checksum != 31616 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_publish_with_params_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_remove()
		})
		if checksum != 46627 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_remove: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_bindingssessioncontext_remove_async()
		})
		if checksum != 40982 {
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
		if checksum != 40643 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_bindingssessioncontext_source: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_fficompletionhandle_wait()
		})
		if checksum != 40168 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_fficompletionhandle_wait: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_fficompletionhandle_wait_async()
		})
		if checksum != 15030 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_fficompletionhandle_wait_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_fficompletionhandle_wait_for()
		})
		if checksum != 59303 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_fficompletionhandle_wait_for: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_bindings_checksum_method_fficompletionhandle_wait_for_async()
		})
		if checksum != 30150 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_bindings_checksum_method_fficompletionhandle_wait_for_async: UniFFI API checksum mismatch")
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
	CreateSession(config SessionConfig, destination Name) (*BindingsSessionContext, error)
	// Create a new session (async version)
	//
	// **Auto-waits for session establishment:** This method automatically waits for the
	// session handshake to complete before returning. For point-to-point sessions, this
	// ensures the remote peer has acknowledged the session. For multicast sessions, this
	// ensures the initial setup is complete.
	CreateSessionAsync(config SessionConfig, destination Name) (*BindingsSessionContext, error)
	// Delete a session (blocking version for FFI)
	DeleteSession(session *BindingsSessionContext) error
	// Delete a session (async version)
	DeleteSessionAsync(session *BindingsSessionContext) error
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
	ListenForSession(timeoutMs *uint32) (*BindingsSessionContext, error)
	// Listen for incoming sessions (async version)
	ListenForSessionAsync(timeoutMs *uint32) (*BindingsSessionContext, error)
	// Get the app name
	Name() Name
	// Remove a route (blocking version for FFI)
	RemoveRoute(name Name, connectionId uint64) error
	// Remove a route (async version)
	RemoveRouteAsync(name Name, connectionId uint64) error
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
	SetRoute(name Name, connectionId uint64) error
	// Set a route to a name for a specific connection (async version)
	SetRouteAsync(name Name, connectionId uint64) error
	// Stop a running SLIM server (blocking version for FFI)
	//
	// # Arguments
	// * `endpoint` - The endpoint address of the server to stop (e.g., "127.0.0.1:12345")
	//
	// # Returns
	// * `Ok(())` - Server stopped successfully
	// * `Err(SlimError)` - If server not found or stop fails
	StopServer(endpoint string) error
	// Subscribe to a name (blocking version for FFI)
	Subscribe(name Name, connectionId *uint64) error
	// Subscribe to a name (async version)
	SubscribeAsync(name Name, connectionId *uint64) error
	// Unsubscribe from a name (blocking version for FFI)
	Unsubscribe(name Name, connectionId *uint64) error
	// Unsubscribe from a name (async version)
	UnsubscribeAsync(name Name, connectionId *uint64) error
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
func (_self *BindingsAdapter) CreateSession(config SessionConfig, destination Name) (*BindingsSessionContext, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_method_bindingsadapter_create_session(
			_pointer, FfiConverterSessionConfigINSTANCE.Lower(config), FfiConverterNameINSTANCE.Lower(destination), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *BindingsSessionContext
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterBindingsSessionContextINSTANCE.Lift(_uniffiRV), nil
	}
}

// Create a new session (async version)
//
// **Auto-waits for session establishment:** This method automatically waits for the
// session handshake to complete before returning. For point-to-point sessions, this
// ensures the remote peer has acknowledged the session. For multicast sessions, this
// ensures the initial setup is complete.
func (_self *BindingsAdapter) CreateSessionAsync(config SessionConfig, destination Name) (*BindingsSessionContext, error) {
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
		C.uniffi_slim_bindings_fn_method_bindingsadapter_create_session_async(
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

// Delete a session (blocking version for FFI)
func (_self *BindingsAdapter) DeleteSession(session *BindingsSessionContext) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingsadapter_delete_session(
			_pointer, FfiConverterBindingsSessionContextINSTANCE.Lower(session), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Delete a session (async version)
func (_self *BindingsAdapter) DeleteSessionAsync(session *BindingsSessionContext) error {
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
		C.uniffi_slim_bindings_fn_method_bindingsadapter_delete_session_async(
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
func (_self *BindingsAdapter) ListenForSession(timeoutMs *uint32) (*BindingsSessionContext, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_method_bindingsadapter_listen_for_session(
			_pointer, FfiConverterOptionalUint32INSTANCE.Lower(timeoutMs), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *BindingsSessionContext
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterBindingsSessionContextINSTANCE.Lift(_uniffiRV), nil
	}
}

// Listen for incoming sessions (async version)
func (_self *BindingsAdapter) ListenForSessionAsync(timeoutMs *uint32) (*BindingsSessionContext, error) {
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
			_pointer, FfiConverterOptionalUint32INSTANCE.Lower(timeoutMs)),
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
func (_self *BindingsAdapter) Name() Name {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterNameINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_bindings_fn_method_bindingsadapter_name(
				_pointer, _uniffiStatus),
		}
	}))
}

// Remove a route (blocking version for FFI)
func (_self *BindingsAdapter) RemoveRoute(name Name, connectionId uint64) error {
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
func (_self *BindingsAdapter) RemoveRouteAsync(name Name, connectionId uint64) error {
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
func (_self *BindingsAdapter) SetRoute(name Name, connectionId uint64) error {
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
func (_self *BindingsAdapter) SetRouteAsync(name Name, connectionId uint64) error {
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

// Subscribe to a name (blocking version for FFI)
func (_self *BindingsAdapter) Subscribe(name Name, connectionId *uint64) error {
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
func (_self *BindingsAdapter) SubscribeAsync(name Name, connectionId *uint64) error {
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
func (_self *BindingsAdapter) Unsubscribe(name Name, connectionId *uint64) error {
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
func (_self *BindingsAdapter) UnsubscribeAsync(name Name, connectionId *uint64) error {
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
	// Get the destination name for this session
	Destination() (Name, error)
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
	// Invite a participant to the session (blocking version for FFI)
	//
	// **Auto-waits for completion:** This method automatically waits for the
	// invitation to be sent and acknowledged before returning.
	Invite(participant Name) error
	// Invite a participant to the session (async version)
	//
	// **Auto-waits for completion:** This method automatically waits for the
	// invitation to be sent and acknowledged before returning.
	InviteAsync(participant Name) error
	// Check if this session is the initiator
	IsInitiator() (bool, error)
	// Publish a message to the session's destination (fire-and-forget, blocking version)
	//
	// This is the simple "fire-and-forget" API that most users want.
	// The message is queued for sending and this method returns immediately without
	// waiting for delivery confirmation.
	//
	// **When to use:** Most common use case where you don't need delivery confirmation.
	//
	// **When not to use:** If you need to ensure the message was delivered, use
	// `publish_with_completion()` instead.
	//
	// # Arguments
	// * `data` - The message payload bytes
	// * `payload_type` - Optional content type identifier
	// * `metadata` - Optional key-value metadata pairs
	//
	// # Returns
	// * `Ok(())` - Message queued successfully
	// * `Err(SlimError)` - If publishing fails
	Publish(data []byte, payloadType *string, metadata *map[string]string) error
	// Publish a message to the session's destination (fire-and-forget, async version)
	PublishAsync(data []byte, payloadType *string, metadata *map[string]string) error
	// Publish a reply message to the originator of a received message (blocking version for FFI)
	//
	// This method uses the routing information from a previously received message
	// to send a reply back to the sender. This is the preferred way to implement
	// request/reply patterns.
	//
	// # Arguments
	// * `message_context` - Context from a message received via `get_message()`
	// * `data` - The reply payload bytes
	// * `payload_type` - Optional content type identifier
	// * `metadata` - Optional key-value metadata pairs
	//
	// # Returns
	// * `Ok(())` on success
	// * `Err(SlimError)` if publishing fails
	PublishTo(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) error
	// Publish a reply message (fire-and-forget, async version)
	PublishToAsync(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) error
	// Publish a reply message with delivery confirmation (blocking version)
	//
	// Similar to `publish_with_completion()` but for reply messages.
	// Returns a completion handle to await delivery confirmation.
	//
	// # Arguments
	// * `message_context` - Context from a message received via `get_message()`
	// * `data` - The reply payload bytes
	// * `payload_type` - Optional content type identifier
	// * `metadata` - Optional key-value metadata pairs
	//
	// # Returns
	// * `Ok(FfiCompletionHandle)` - Handle to await delivery confirmation
	// * `Err(SlimError)` - If publishing fails
	PublishToWithCompletion(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) (*FfiCompletionHandle, error)
	// Publish a reply message with delivery confirmation (async version)
	PublishToWithCompletionAsync(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) (*FfiCompletionHandle, error)
	// Publish a message with delivery confirmation (blocking version)
	//
	// This variant returns a `FfiCompletionHandle` that can be awaited to ensure
	// the message was delivered successfully. Use this when you need reliable
	// delivery confirmation.
	//
	// **When to use:** Critical messages where you need delivery confirmation.
	//
	// # Arguments
	// * `data` - The message payload bytes
	// * `payload_type` - Optional content type identifier
	// * `metadata` - Optional key-value metadata pairs
	//
	// # Returns
	// * `Ok(FfiCompletionHandle)` - Handle to await delivery confirmation
	// * `Err(SlimError)` - If publishing fails
	//
	// # Example
	// ```ignore
	// let completion = session.publish_with_completion(data, None, None)?;
	// completion.wait()?; // Blocks until message is delivered
	// ```
	PublishWithCompletion(data []byte, payloadType *string, metadata *map[string]string) (*FfiCompletionHandle, error)
	// Publish a message with delivery confirmation (async version)
	PublishWithCompletionAsync(data []byte, payloadType *string, metadata *map[string]string) (*FfiCompletionHandle, error)
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
	PublishWithParams(destination Name, fanout uint32, data []byte, connectionOut *uint64, payloadType *string, metadata *map[string]string) error
	// Low-level publish with full control (async version)
	PublishWithParamsAsync(destination Name, fanout uint32, data []byte, connectionOut *uint64, payloadType *string, metadata *map[string]string) error
	// Remove a participant from the session (blocking version for FFI)
	//
	// **Auto-waits for completion:** This method automatically waits for the
	// removal to be processed and acknowledged before returning.
	Remove(participant Name) error
	// Remove a participant from the session (async version)
	//
	// **Auto-waits for completion:** This method automatically waits for the
	// removal to be processed and acknowledged before returning.
	RemoveAsync(participant Name) error
	// Get the session ID
	SessionId() (uint32, error)
	// Get the session type (PointToPoint or Group)
	SessionType() (SessionType, error)
	// Get the source name for this session
	Source() (Name, error)
}

// Session context for language bindings (UniFFI-compatible)
//
// Wraps the session context with proper async access patterns for message reception.
// Provides both synchronous (blocking) and asynchronous methods for FFI compatibility.
type BindingsSessionContext struct {
	ffiObject FfiObject
}

// Get the destination name for this session
func (_self *BindingsSessionContext) Destination() (Name, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_bindings_fn_method_bindingssessioncontext_destination(
				_pointer, _uniffiStatus),
		}
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue Name
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

// Invite a participant to the session (blocking version for FFI)
//
// **Auto-waits for completion:** This method automatically waits for the
// invitation to be sent and acknowledged before returning.
func (_self *BindingsSessionContext) Invite(participant Name) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_invite(
			_pointer, FfiConverterNameINSTANCE.Lower(participant), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Invite a participant to the session (async version)
//
// **Auto-waits for completion:** This method automatically waits for the
// invitation to be sent and acknowledged before returning.
func (_self *BindingsSessionContext) InviteAsync(participant Name) error {
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
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_invite_async(
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

// Publish a message to the session's destination (fire-and-forget, blocking version)
//
// This is the simple "fire-and-forget" API that most users want.
// The message is queued for sending and this method returns immediately without
// waiting for delivery confirmation.
//
// **When to use:** Most common use case where you don't need delivery confirmation.
//
// **When not to use:** If you need to ensure the message was delivered, use
// `publish_with_completion()` instead.
//
// # Arguments
// * `data` - The message payload bytes
// * `payload_type` - Optional content type identifier
// * `metadata` - Optional key-value metadata pairs
//
// # Returns
// * `Ok(())` - Message queued successfully
// * `Err(SlimError)` - If publishing fails
func (_self *BindingsSessionContext) Publish(data []byte, payloadType *string, metadata *map[string]string) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish(
			_pointer, FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Publish a message to the session's destination (fire-and-forget, async version)
func (_self *BindingsSessionContext) PublishAsync(data []byte, payloadType *string, metadata *map[string]string) error {
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
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_async(
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

// Publish a reply message to the originator of a received message (blocking version for FFI)
//
// This method uses the routing information from a previously received message
// to send a reply back to the sender. This is the preferred way to implement
// request/reply patterns.
//
// # Arguments
// * `message_context` - Context from a message received via `get_message()`
// * `data` - The reply payload bytes
// * `payload_type` - Optional content type identifier
// * `metadata` - Optional key-value metadata pairs
//
// # Returns
// * `Ok(())` on success
// * `Err(SlimError)` if publishing fails
func (_self *BindingsSessionContext) PublishTo(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_to(
			_pointer, FfiConverterMessageContextINSTANCE.Lower(messageContext), FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Publish a reply message (fire-and-forget, async version)
func (_self *BindingsSessionContext) PublishToAsync(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) error {
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
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_to_async(
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

// Publish a reply message with delivery confirmation (blocking version)
//
// Similar to `publish_with_completion()` but for reply messages.
// Returns a completion handle to await delivery confirmation.
//
// # Arguments
// * `message_context` - Context from a message received via `get_message()`
// * `data` - The reply payload bytes
// * `payload_type` - Optional content type identifier
// * `metadata` - Optional key-value metadata pairs
//
// # Returns
// * `Ok(FfiCompletionHandle)` - Handle to await delivery confirmation
// * `Err(SlimError)` - If publishing fails
func (_self *BindingsSessionContext) PublishToWithCompletion(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) (*FfiCompletionHandle, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_to_with_completion(
			_pointer, FfiConverterMessageContextINSTANCE.Lower(messageContext), FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *FfiCompletionHandle
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterFfiCompletionHandleINSTANCE.Lift(_uniffiRV), nil
	}
}

// Publish a reply message with delivery confirmation (async version)
func (_self *BindingsSessionContext) PublishToWithCompletionAsync(messageContext MessageContext, data []byte, payloadType *string, metadata *map[string]string) (*FfiCompletionHandle, error) {
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
		func(ffi unsafe.Pointer) *FfiCompletionHandle {
			return FfiConverterFfiCompletionHandleINSTANCE.Lift(ffi)
		},
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_to_with_completion_async(
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

// Publish a message with delivery confirmation (blocking version)
//
// This variant returns a `FfiCompletionHandle` that can be awaited to ensure
// the message was delivered successfully. Use this when you need reliable
// delivery confirmation.
//
// **When to use:** Critical messages where you need delivery confirmation.
//
// # Arguments
// * `data` - The message payload bytes
// * `payload_type` - Optional content type identifier
// * `metadata` - Optional key-value metadata pairs
//
// # Returns
// * `Ok(FfiCompletionHandle)` - Handle to await delivery confirmation
// * `Err(SlimError)` - If publishing fails
//
// # Example
// ```ignore
// let completion = session.publish_with_completion(data, None, None)?;
// completion.wait()?; // Blocks until message is delivered
// ```
func (_self *BindingsSessionContext) PublishWithCompletion(data []byte, payloadType *string, metadata *map[string]string) (*FfiCompletionHandle, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_with_completion(
			_pointer, FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *FfiCompletionHandle
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterFfiCompletionHandleINSTANCE.Lift(_uniffiRV), nil
	}
}

// Publish a message with delivery confirmation (async version)
func (_self *BindingsSessionContext) PublishWithCompletionAsync(data []byte, payloadType *string, metadata *map[string]string) (*FfiCompletionHandle, error) {
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
		func(ffi unsafe.Pointer) *FfiCompletionHandle {
			return FfiConverterFfiCompletionHandleINSTANCE.Lift(ffi)
		},
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_publish_with_completion_async(
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
func (_self *BindingsSessionContext) PublishWithParams(destination Name, fanout uint32, data []byte, connectionOut *uint64, payloadType *string, metadata *map[string]string) error {
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
func (_self *BindingsSessionContext) PublishWithParamsAsync(destination Name, fanout uint32, data []byte, connectionOut *uint64, payloadType *string, metadata *map[string]string) error {
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

// Remove a participant from the session (blocking version for FFI)
//
// **Auto-waits for completion:** This method automatically waits for the
// removal to be processed and acknowledged before returning.
func (_self *BindingsSessionContext) Remove(participant Name) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_remove(
			_pointer, FfiConverterNameINSTANCE.Lower(participant), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Remove a participant from the session (async version)
//
// **Auto-waits for completion:** This method automatically waits for the
// removal to be processed and acknowledged before returning.
func (_self *BindingsSessionContext) RemoveAsync(participant Name) error {
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
		C.uniffi_slim_bindings_fn_method_bindingssessioncontext_remove_async(
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
func (_self *BindingsSessionContext) Source() (Name, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_bindings_fn_method_bindingssessioncontext_source(
				_pointer, _uniffiStatus),
		}
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue Name
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
// # Design Note
// Since Rust futures can only be polled once to completion, this handle uses
// a shared receiver that can only be consumed once. Attempting to wait multiple
// times on the same handle will return an error.
//
// # Examples
//
// Basic usage:
// ```ignore
// let completion = session.publish_with_completion(data, None, None)?;
// completion.wait()?; // Wait for delivery confirmation
// ```
type FfiCompletionHandleInterface interface {
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
// # Design Note
// Since Rust futures can only be polled once to completion, this handle uses
// a shared receiver that can only be consumed once. Attempting to wait multiple
// times on the same handle will return an error.
//
// # Examples
//
// Basic usage:
// ```ignore
// let completion = session.publish_with_completion(data, None, None)?;
// completion.wait()?; // Wait for delivery confirmation
// ```
type FfiCompletionHandle struct {
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
func (_self *FfiCompletionHandle) Wait() error {
	_pointer := _self.ffiObject.incrementPointer("*FfiCompletionHandle")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_fficompletionhandle_wait(
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
func (_self *FfiCompletionHandle) WaitAsync() error {
	_pointer := _self.ffiObject.incrementPointer("*FfiCompletionHandle")
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
		C.uniffi_slim_bindings_fn_method_fficompletionhandle_wait_async(
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
func (_self *FfiCompletionHandle) WaitFor(timeout time.Duration) error {
	_pointer := _self.ffiObject.incrementPointer("*FfiCompletionHandle")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_bindings_fn_method_fficompletionhandle_wait_for(
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
func (_self *FfiCompletionHandle) WaitForAsync(timeout time.Duration) error {
	_pointer := _self.ffiObject.incrementPointer("*FfiCompletionHandle")
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
		C.uniffi_slim_bindings_fn_method_fficompletionhandle_wait_for_async(
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
func (object *FfiCompletionHandle) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterFfiCompletionHandle struct{}

var FfiConverterFfiCompletionHandleINSTANCE = FfiConverterFfiCompletionHandle{}

func (c FfiConverterFfiCompletionHandle) Lift(pointer unsafe.Pointer) *FfiCompletionHandle {
	result := &FfiCompletionHandle{
		newFfiObject(
			pointer,
			func(pointer unsafe.Pointer, status *C.RustCallStatus) unsafe.Pointer {
				return C.uniffi_slim_bindings_fn_clone_fficompletionhandle(pointer, status)
			},
			func(pointer unsafe.Pointer, status *C.RustCallStatus) {
				C.uniffi_slim_bindings_fn_free_fficompletionhandle(pointer, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*FfiCompletionHandle).Destroy)
	return result
}

func (c FfiConverterFfiCompletionHandle) Read(reader io.Reader) *FfiCompletionHandle {
	return c.Lift(unsafe.Pointer(uintptr(readUint64(reader))))
}

func (c FfiConverterFfiCompletionHandle) Lower(value *FfiCompletionHandle) unsafe.Pointer {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the pointer will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked pointer.
	pointer := value.ffiObject.incrementPointer("*FfiCompletionHandle")
	defer value.ffiObject.decrementPointer()
	return pointer

}

func (c FfiConverterFfiCompletionHandle) Write(writer io.Writer, value *FfiCompletionHandle) {
	writeUint64(writer, uint64(uintptr(c.Lower(value))))
}

type FfiDestroyerFfiCompletionHandle struct{}

func (_ FfiDestroyerFfiCompletionHandle) Destroy(value *FfiCompletionHandle) {
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
	Endpoint string
	Tls      TlsConfig
}

func (r *ClientConfig) Destroy() {
	FfiDestroyerString{}.Destroy(r.Endpoint)
	FfiDestroyerTlsConfig{}.Destroy(r.Tls)
}

type FfiConverterClientConfig struct{}

var FfiConverterClientConfigINSTANCE = FfiConverterClientConfig{}

func (c FfiConverterClientConfig) Lift(rb RustBufferI) ClientConfig {
	return LiftFromRustBuffer[ClientConfig](c, rb)
}

func (c FfiConverterClientConfig) Read(reader io.Reader) ClientConfig {
	return ClientConfig{
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterTlsConfigINSTANCE.Read(reader),
	}
}

func (c FfiConverterClientConfig) Lower(value ClientConfig) C.RustBuffer {
	return LowerIntoRustBuffer[ClientConfig](c, value)
}

func (c FfiConverterClientConfig) Write(writer io.Writer, value ClientConfig) {
	FfiConverterStringINSTANCE.Write(writer, value.Endpoint)
	FfiConverterTlsConfigINSTANCE.Write(writer, value.Tls)
}

type FfiDestroyerClientConfig struct{}

func (_ FfiDestroyerClientConfig) Destroy(value ClientConfig) {
	value.Destroy()
}

// Generic message context for language bindings (UniFFI-compatible)
//
// Provides routing and descriptive metadata needed for replying,
// auditing, and instrumentation across different language bindings.
// This type is exported to foreign languages via UniFFI.
type MessageContext struct {
	// Fully-qualified sender identity
	SourceName Name
	// Fully-qualified destination identity (may be empty for broadcast/group scenarios)
	DestinationName *Name
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

// Name type for SLIM (Secure Low-Latency Interactive Messaging)
type Name struct {
	Components []string
	Id         *uint64
}

func (r *Name) Destroy() {
	FfiDestroyerSequenceString{}.Destroy(r.Components)
	FfiDestroyerOptionalUint64{}.Destroy(r.Id)
}

type FfiConverterName struct{}

var FfiConverterNameINSTANCE = FfiConverterName{}

func (c FfiConverterName) Lift(rb RustBufferI) Name {
	return LiftFromRustBuffer[Name](c, rb)
}

func (c FfiConverterName) Read(reader io.Reader) Name {
	return Name{
		FfiConverterSequenceStringINSTANCE.Read(reader),
		FfiConverterOptionalUint64INSTANCE.Read(reader),
	}
}

func (c FfiConverterName) Lower(value Name) C.RustBuffer {
	return LowerIntoRustBuffer[Name](c, value)
}

func (c FfiConverterName) Write(writer io.Writer, value Name) {
	FfiConverterSequenceStringINSTANCE.Write(writer, value.Components)
	FfiConverterOptionalUint64INSTANCE.Write(writer, value.Id)
}

type FfiDestroyerName struct{}

func (_ FfiDestroyerName) Destroy(value Name) {
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
	Endpoint string
	Tls      TlsConfig
}

func (r *ServerConfig) Destroy() {
	FfiDestroyerString{}.Destroy(r.Endpoint)
	FfiDestroyerTlsConfig{}.Destroy(r.Tls)
}

type FfiConverterServerConfig struct{}

var FfiConverterServerConfigINSTANCE = FfiConverterServerConfig{}

func (c FfiConverterServerConfig) Lift(rb RustBufferI) ServerConfig {
	return LiftFromRustBuffer[ServerConfig](c, rb)
}

func (c FfiConverterServerConfig) Read(reader io.Reader) ServerConfig {
	return ServerConfig{
		FfiConverterStringINSTANCE.Read(reader),
		FfiConverterTlsConfigINSTANCE.Read(reader),
	}
}

func (c FfiConverterServerConfig) Lower(value ServerConfig) C.RustBuffer {
	return LowerIntoRustBuffer[ServerConfig](c, value)
}

func (c FfiConverterServerConfig) Write(writer io.Writer, value ServerConfig) {
	FfiConverterStringINSTANCE.Write(writer, value.Endpoint)
	FfiConverterTlsConfigINSTANCE.Write(writer, value.Tls)
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
	IntervalMs *uint64
	// Whether this endpoint is the session initiator
	Initiator bool
	// Custom metadata key-value pairs for the session
	Metadata map[string]string
}

func (r *SessionConfig) Destroy() {
	FfiDestroyerSessionType{}.Destroy(r.SessionType)
	FfiDestroyerBool{}.Destroy(r.EnableMls)
	FfiDestroyerOptionalUint32{}.Destroy(r.MaxRetries)
	FfiDestroyerOptionalUint64{}.Destroy(r.IntervalMs)
	FfiDestroyerBool{}.Destroy(r.Initiator)
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
		FfiConverterOptionalUint64INSTANCE.Read(reader),
		FfiConverterBoolINSTANCE.Read(reader),
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
	FfiConverterOptionalUint64INSTANCE.Write(writer, value.IntervalMs)
	FfiConverterBoolINSTANCE.Write(writer, value.Initiator)
	FfiConverterMapStringStringINSTANCE.Write(writer, value.Metadata)
}

type FfiDestroyerSessionConfig struct{}

func (_ FfiDestroyerSessionConfig) Destroy(value SessionConfig) {
	value.Destroy()
}

// TLS configuration for server/client
type TlsConfig struct {
	// Disable TLS entirely (plain text connection)
	Insecure bool
	// Skip server certificate verification (client only, enables TLS but doesn't verify certs)
	// WARNING: Only use for testing - insecure in production!
	InsecureSkipVerify *bool
	// Path to certificate file (PEM format)
	CertFile *string
	// Path to private key file (PEM format)
	KeyFile *string
	// Path to CA certificate file (PEM format) for verifying peer certificates
	CaFile *string
	// TLS version to use: "tls1.2" or "tls1.3" (default: "tls1.3")
	TlsVersion *string
	// Include system CA certificates pool (default: false)
	IncludeSystemCaCertsPool *bool
}

func (r *TlsConfig) Destroy() {
	FfiDestroyerBool{}.Destroy(r.Insecure)
	FfiDestroyerOptionalBool{}.Destroy(r.InsecureSkipVerify)
	FfiDestroyerOptionalString{}.Destroy(r.CertFile)
	FfiDestroyerOptionalString{}.Destroy(r.KeyFile)
	FfiDestroyerOptionalString{}.Destroy(r.CaFile)
	FfiDestroyerOptionalString{}.Destroy(r.TlsVersion)
	FfiDestroyerOptionalBool{}.Destroy(r.IncludeSystemCaCertsPool)
}

type FfiConverterTlsConfig struct{}

var FfiConverterTlsConfigINSTANCE = FfiConverterTlsConfig{}

func (c FfiConverterTlsConfig) Lift(rb RustBufferI) TlsConfig {
	return LiftFromRustBuffer[TlsConfig](c, rb)
}

func (c FfiConverterTlsConfig) Read(reader io.Reader) TlsConfig {
	return TlsConfig{
		FfiConverterBoolINSTANCE.Read(reader),
		FfiConverterOptionalBoolINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterOptionalStringINSTANCE.Read(reader),
		FfiConverterOptionalBoolINSTANCE.Read(reader),
	}
}

func (c FfiConverterTlsConfig) Lower(value TlsConfig) C.RustBuffer {
	return LowerIntoRustBuffer[TlsConfig](c, value)
}

func (c FfiConverterTlsConfig) Write(writer io.Writer, value TlsConfig) {
	FfiConverterBoolINSTANCE.Write(writer, value.Insecure)
	FfiConverterOptionalBoolINSTANCE.Write(writer, value.InsecureSkipVerify)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.CertFile)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.KeyFile)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.CaFile)
	FfiConverterOptionalStringINSTANCE.Write(writer, value.TlsVersion)
	FfiConverterOptionalBoolINSTANCE.Write(writer, value.IncludeSystemCaCertsPool)
}

type FfiDestroyerTlsConfig struct{}

func (_ FfiDestroyerTlsConfig) Destroy(value TlsConfig) {
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

type FfiConverterOptionalBool struct{}

var FfiConverterOptionalBoolINSTANCE = FfiConverterOptionalBool{}

func (c FfiConverterOptionalBool) Lift(rb RustBufferI) *bool {
	return LiftFromRustBuffer[*bool](c, rb)
}

func (_ FfiConverterOptionalBool) Read(reader io.Reader) *bool {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterBoolINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalBool) Lower(value *bool) C.RustBuffer {
	return LowerIntoRustBuffer[*bool](c, value)
}

func (_ FfiConverterOptionalBool) Write(writer io.Writer, value *bool) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterBoolINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalBool struct{}

func (_ FfiDestroyerOptionalBool) Destroy(value *bool) {
	if value != nil {
		FfiDestroyerBool{}.Destroy(*value)
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

type FfiConverterOptionalName struct{}

var FfiConverterOptionalNameINSTANCE = FfiConverterOptionalName{}

func (c FfiConverterOptionalName) Lift(rb RustBufferI) *Name {
	return LiftFromRustBuffer[*Name](c, rb)
}

func (_ FfiConverterOptionalName) Read(reader io.Reader) *Name {
	if readInt8(reader) == 0 {
		return nil
	}
	temp := FfiConverterNameINSTANCE.Read(reader)
	return &temp
}

func (c FfiConverterOptionalName) Lower(value *Name) C.RustBuffer {
	return LowerIntoRustBuffer[*Name](c, value)
}

func (_ FfiConverterOptionalName) Write(writer io.Writer, value *Name) {
	if value == nil {
		writeInt8(writer, 0)
	} else {
		writeInt8(writer, 1)
		FfiConverterNameINSTANCE.Write(writer, *value)
	}
}

type FfiDestroyerOptionalName struct{}

func (_ FfiDestroyerOptionalName) Destroy(value *Name) {
	if value != nil {
		FfiDestroyerName{}.Destroy(*value)
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
func CreateAppWithSecret(appName Name, sharedSecret string) (*BindingsAdapter, error) {
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
