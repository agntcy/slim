package slim_service

// #include <slim_service.h>
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
		C.ffi_slim_service_rustbuffer_free(cb.inner, status)
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
		return C.ffi_slim_service_rustbuffer_from_bytes(foreign, status)
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
		return C.ffi_slim_service_uniffi_contract_version()
	})
	if bindingsContractVersion != int(scaffoldingContractVersion) {
		// If this happens try cleaning and rebuilding your project
		panic("slim_service: UniFFI contract version mismatch")
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_func_create_app_with_secret()
		})
		if checksum != 50906 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_func_create_app_with_secret: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_func_get_version()
		})
		if checksum != 53523 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_func_get_version: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_func_initialize_crypto()
		})
		if checksum != 55801 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_func_initialize_crypto: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_connect()
		})
		if checksum != 48008 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_connect: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_connect_async()
		})
		if checksum != 42152 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_connect_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_create_session()
		})
		if checksum != 48429 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_create_session: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_create_session_async()
		})
		if checksum != 39669 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_create_session_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_delete_session()
		})
		if checksum != 11000 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_delete_session: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_disconnect()
		})
		if checksum != 1820 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_disconnect: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_disconnect_async()
		})
		if checksum != 60557 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_disconnect_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_id()
		})
		if checksum != 374 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_id: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_listen_for_session()
		})
		if checksum != 52700 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_listen_for_session: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_listen_for_session_async()
		})
		if checksum != 27163 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_listen_for_session_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_name()
		})
		if checksum != 23815 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_name: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_remove_route()
		})
		if checksum != 7326 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_remove_route: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_remove_route_async()
		})
		if checksum != 56712 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_remove_route_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_run_server()
		})
		if checksum != 16193 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_run_server: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_run_server_async()
		})
		if checksum != 764 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_run_server_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_set_route()
		})
		if checksum != 18642 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_set_route: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_set_route_async()
		})
		if checksum != 58142 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_set_route_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_subscribe()
		})
		if checksum != 1673 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_subscribe: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_subscribe_async()
		})
		if checksum != 11535 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_subscribe_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_unsubscribe()
		})
		if checksum != 13215 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_unsubscribe: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_bindingsadapter_unsubscribe_async()
		})
		if checksum != 43766 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_bindingsadapter_unsubscribe_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_ffisessioncontext_destination()
		})
		if checksum != 21116 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_ffisessioncontext_destination: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_ffisessioncontext_get_message()
		})
		if checksum != 59165 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_ffisessioncontext_get_message: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_ffisessioncontext_get_message_async()
		})
		if checksum != 58295 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_ffisessioncontext_get_message_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_ffisessioncontext_invite()
		})
		if checksum != 55427 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_ffisessioncontext_invite: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_ffisessioncontext_invite_async()
		})
		if checksum != 33273 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_ffisessioncontext_invite_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_ffisessioncontext_is_initiator()
		})
		if checksum != 50359 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_ffisessioncontext_is_initiator: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_ffisessioncontext_publish()
		})
		if checksum != 26365 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_ffisessioncontext_publish: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_ffisessioncontext_publish_async()
		})
		if checksum != 24604 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_ffisessioncontext_publish_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_ffisessioncontext_remove()
		})
		if checksum != 50535 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_ffisessioncontext_remove: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_ffisessioncontext_remove_async()
		})
		if checksum != 29332 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_ffisessioncontext_remove_async: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_ffisessioncontext_session_id()
		})
		if checksum != 17475 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_ffisessioncontext_session_id: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_ffisessioncontext_session_type()
		})
		if checksum != 18788 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_ffisessioncontext_session_type: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_service_checksum_method_ffisessioncontext_source()
		})
		if checksum != 14596 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_service: uniffi_slim_service_checksum_method_ffisessioncontext_source: UniFFI API checksum mismatch")
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
	CreateSession(config SessionConfig, destination Name) (*FfiSessionContext, error)
	// Create a new session (async version)
	CreateSessionAsync(config SessionConfig, destination Name) (*FfiSessionContext, error)
	// Delete a session (synchronous - no async version needed)
	DeleteSession(session *FfiSessionContext) error
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
	ListenForSession(timeoutMs *uint32) (*FfiSessionContext, error)
	// Listen for incoming sessions (async version)
	ListenForSessionAsync(timeoutMs *uint32) (*FfiSessionContext, error)
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
		return C.uniffi_slim_service_fn_method_bindingsadapter_connect(
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
			res := C.ffi_slim_service_rust_future_complete_u64(handle, status)
			return res
		},
		// liftFn
		func(ffi C.uint64_t) uint64 {
			return FfiConverterUint64INSTANCE.Lift(ffi)
		},
		C.uniffi_slim_service_fn_method_bindingsadapter_connect_async(
			_pointer, FfiConverterClientConfigINSTANCE.Lower(config)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_service_rust_future_poll_u64(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_service_rust_future_free_u64(handle)
		},
	)

	return res, err
}

// Create a new session (blocking version for FFI)
func (_self *BindingsAdapter) CreateSession(config SessionConfig, destination Name) (*FfiSessionContext, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_service_fn_method_bindingsadapter_create_session(
			_pointer, FfiConverterSessionConfigINSTANCE.Lower(config), FfiConverterNameINSTANCE.Lower(destination), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *FfiSessionContext
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterFfiSessionContextINSTANCE.Lift(_uniffiRV), nil
	}
}

// Create a new session (async version)
func (_self *BindingsAdapter) CreateSessionAsync(config SessionConfig, destination Name) (*FfiSessionContext, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) unsafe.Pointer {
			res := C.ffi_slim_service_rust_future_complete_pointer(handle, status)
			return res
		},
		// liftFn
		func(ffi unsafe.Pointer) *FfiSessionContext {
			return FfiConverterFfiSessionContextINSTANCE.Lift(ffi)
		},
		C.uniffi_slim_service_fn_method_bindingsadapter_create_session_async(
			_pointer, FfiConverterSessionConfigINSTANCE.Lower(config), FfiConverterNameINSTANCE.Lower(destination)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_service_rust_future_poll_pointer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_service_rust_future_free_pointer(handle)
		},
	)

	return res, err
}

// Delete a session (synchronous - no async version needed)
func (_self *BindingsAdapter) DeleteSession(session *FfiSessionContext) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_service_fn_method_bindingsadapter_delete_session(
			_pointer, FfiConverterFfiSessionContextINSTANCE.Lower(session), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
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
		C.uniffi_slim_service_fn_method_bindingsadapter_disconnect(
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
			C.ffi_slim_service_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_service_fn_method_bindingsadapter_disconnect_async(
			_pointer, FfiConverterUint64INSTANCE.Lower(connectionId)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_service_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_service_rust_future_free_void(handle)
		},
	)

	return err
}

// Get the app ID (derived from name)
func (_self *BindingsAdapter) Id() uint64 {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterUint64INSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_slim_service_fn_method_bindingsadapter_id(
			_pointer, _uniffiStatus)
	}))
}

// Listen for incoming sessions (blocking version for FFI)
func (_self *BindingsAdapter) ListenForSession(timeoutMs *uint32) (*FfiSessionContext, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_service_fn_method_bindingsadapter_listen_for_session(
			_pointer, FfiConverterOptionalUint32INSTANCE.Lower(timeoutMs), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *FfiSessionContext
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterFfiSessionContextINSTANCE.Lift(_uniffiRV), nil
	}
}

// Listen for incoming sessions (async version)
func (_self *BindingsAdapter) ListenForSessionAsync(timeoutMs *uint32) (*FfiSessionContext, error) {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) unsafe.Pointer {
			res := C.ffi_slim_service_rust_future_complete_pointer(handle, status)
			return res
		},
		// liftFn
		func(ffi unsafe.Pointer) *FfiSessionContext {
			return FfiConverterFfiSessionContextINSTANCE.Lift(ffi)
		},
		C.uniffi_slim_service_fn_method_bindingsadapter_listen_for_session_async(
			_pointer, FfiConverterOptionalUint32INSTANCE.Lower(timeoutMs)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_service_rust_future_poll_pointer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_service_rust_future_free_pointer(handle)
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
			inner: C.uniffi_slim_service_fn_method_bindingsadapter_name(
				_pointer, _uniffiStatus),
		}
	}))
}

// Remove a route (blocking version for FFI)
func (_self *BindingsAdapter) RemoveRoute(name Name, connectionId uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_service_fn_method_bindingsadapter_remove_route(
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
			C.ffi_slim_service_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_service_fn_method_bindingsadapter_remove_route_async(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterUint64INSTANCE.Lower(connectionId)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_service_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_service_rust_future_free_void(handle)
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
		C.uniffi_slim_service_fn_method_bindingsadapter_run_server(
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
			C.ffi_slim_service_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_service_fn_method_bindingsadapter_run_server_async(
			_pointer, FfiConverterServerConfigINSTANCE.Lower(config)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_service_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_service_rust_future_free_void(handle)
		},
	)

	return err
}

// Set a route to a name for a specific connection (blocking version for FFI)
func (_self *BindingsAdapter) SetRoute(name Name, connectionId uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_service_fn_method_bindingsadapter_set_route(
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
			C.ffi_slim_service_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_service_fn_method_bindingsadapter_set_route_async(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterUint64INSTANCE.Lower(connectionId)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_service_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_service_rust_future_free_void(handle)
		},
	)

	return err
}

// Subscribe to a name (blocking version for FFI)
func (_self *BindingsAdapter) Subscribe(name Name, connectionId *uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_service_fn_method_bindingsadapter_subscribe(
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
			C.ffi_slim_service_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_service_fn_method_bindingsadapter_subscribe_async(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterOptionalUint64INSTANCE.Lower(connectionId)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_service_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_service_rust_future_free_void(handle)
		},
	)

	return err
}

// Unsubscribe from a name (blocking version for FFI)
func (_self *BindingsAdapter) Unsubscribe(name Name, connectionId *uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*BindingsAdapter")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_service_fn_method_bindingsadapter_unsubscribe(
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
			C.ffi_slim_service_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_service_fn_method_bindingsadapter_unsubscribe_async(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterOptionalUint64INSTANCE.Lower(connectionId)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_service_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_service_rust_future_free_void(handle)
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
				return C.uniffi_slim_service_fn_clone_bindingsadapter(pointer, status)
			},
			func(pointer unsafe.Pointer, status *C.RustCallStatus) {
				C.uniffi_slim_service_fn_free_bindingsadapter(pointer, status)
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

// FFISessionContext represents an active session (FFI-compatible wrapper)
type FfiSessionContextInterface interface {
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
	Invite(participant Name) error
	// Invite a participant to the session (async version)
	InviteAsync(participant Name) error
	// Check if this session is the initiator
	IsInitiator() (bool, error)
	// Publish a message to the session (blocking version for FFI)
	Publish(destination Name, fanout uint32, data []byte, connectionOut *uint64, payloadType *string, metadata *map[string]string) error
	// Publish a message to the session (async version)
	PublishAsync(destination Name, fanout uint32, data []byte, connectionOut *uint64, payloadType *string, metadata *map[string]string) error
	// Remove a participant from the session (blocking version for FFI)
	Remove(participant Name) error
	// Remove a participant from the session (async version)
	RemoveAsync(participant Name) error
	// Get the session ID
	SessionId() (uint32, error)
	// Get the session type (PointToPoint or Multicast)
	SessionType() (SessionType, error)
	// Get the source name for this session
	Source() (Name, error)
}

// FFISessionContext represents an active session (FFI-compatible wrapper)
type FfiSessionContext struct {
	ffiObject FfiObject
}

// Get the destination name for this session
func (_self *FfiSessionContext) Destination() (Name, error) {
	_pointer := _self.ffiObject.incrementPointer("*FfiSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_service_fn_method_ffisessioncontext_destination(
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
func (_self *FfiSessionContext) GetMessage(timeoutMs *uint32) (ReceivedMessage, error) {
	_pointer := _self.ffiObject.incrementPointer("*FfiSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_service_fn_method_ffisessioncontext_get_message(
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
func (_self *FfiSessionContext) GetMessageAsync(timeoutMs *uint32) (ReceivedMessage, error) {
	_pointer := _self.ffiObject.incrementPointer("*FfiSessionContext")
	defer _self.ffiObject.decrementPointer()
	res, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) RustBufferI {
			res := C.ffi_slim_service_rust_future_complete_rust_buffer(handle, status)
			return GoRustBuffer{
				inner: res,
			}
		},
		// liftFn
		func(ffi RustBufferI) ReceivedMessage {
			return FfiConverterReceivedMessageINSTANCE.Lift(ffi)
		},
		C.uniffi_slim_service_fn_method_ffisessioncontext_get_message_async(
			_pointer, FfiConverterOptionalUint32INSTANCE.Lower(timeoutMs)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_service_rust_future_poll_rust_buffer(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_service_rust_future_free_rust_buffer(handle)
		},
	)

	return res, err
}

// Invite a participant to the session (blocking version for FFI)
func (_self *FfiSessionContext) Invite(participant Name) error {
	_pointer := _self.ffiObject.incrementPointer("*FfiSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_service_fn_method_ffisessioncontext_invite(
			_pointer, FfiConverterNameINSTANCE.Lower(participant), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Invite a participant to the session (async version)
func (_self *FfiSessionContext) InviteAsync(participant Name) error {
	_pointer := _self.ffiObject.incrementPointer("*FfiSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_service_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_service_fn_method_ffisessioncontext_invite_async(
			_pointer, FfiConverterNameINSTANCE.Lower(participant)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_service_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_service_rust_future_free_void(handle)
		},
	)

	return err
}

// Check if this session is the initiator
func (_self *FfiSessionContext) IsInitiator() (bool, error) {
	_pointer := _self.ffiObject.incrementPointer("*FfiSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) C.int8_t {
		return C.uniffi_slim_service_fn_method_ffisessioncontext_is_initiator(
			_pointer, _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue bool
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterBoolINSTANCE.Lift(_uniffiRV), nil
	}
}

// Publish a message to the session (blocking version for FFI)
func (_self *FfiSessionContext) Publish(destination Name, fanout uint32, data []byte, connectionOut *uint64, payloadType *string, metadata *map[string]string) error {
	_pointer := _self.ffiObject.incrementPointer("*FfiSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_service_fn_method_ffisessioncontext_publish(
			_pointer, FfiConverterNameINSTANCE.Lower(destination), FfiConverterUint32INSTANCE.Lower(fanout), FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalUint64INSTANCE.Lower(connectionOut), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Publish a message to the session (async version)
func (_self *FfiSessionContext) PublishAsync(destination Name, fanout uint32, data []byte, connectionOut *uint64, payloadType *string, metadata *map[string]string) error {
	_pointer := _self.ffiObject.incrementPointer("*FfiSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_service_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_service_fn_method_ffisessioncontext_publish_async(
			_pointer, FfiConverterNameINSTANCE.Lower(destination), FfiConverterUint32INSTANCE.Lower(fanout), FfiConverterBytesINSTANCE.Lower(data), FfiConverterOptionalUint64INSTANCE.Lower(connectionOut), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_service_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_service_rust_future_free_void(handle)
		},
	)

	return err
}

// Remove a participant from the session (blocking version for FFI)
func (_self *FfiSessionContext) Remove(participant Name) error {
	_pointer := _self.ffiObject.incrementPointer("*FfiSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_service_fn_method_ffisessioncontext_remove(
			_pointer, FfiConverterNameINSTANCE.Lower(participant), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

// Remove a participant from the session (async version)
func (_self *FfiSessionContext) RemoveAsync(participant Name) error {
	_pointer := _self.ffiObject.incrementPointer("*FfiSessionContext")
	defer _self.ffiObject.decrementPointer()
	_, err := uniffiRustCallAsync[SlimError](
		FfiConverterSlimErrorINSTANCE,
		// completeFn
		func(handle C.uint64_t, status *C.RustCallStatus) struct{} {
			C.ffi_slim_service_rust_future_complete_void(handle, status)
			return struct{}{}
		},
		// liftFn
		func(_ struct{}) struct{} { return struct{}{} },
		C.uniffi_slim_service_fn_method_ffisessioncontext_remove_async(
			_pointer, FfiConverterNameINSTANCE.Lower(participant)),
		// pollFn
		func(handle C.uint64_t, continuation C.UniffiRustFutureContinuationCallback, data C.uint64_t) {
			C.ffi_slim_service_rust_future_poll_void(handle, continuation, data)
		},
		// freeFn
		func(handle C.uint64_t) {
			C.ffi_slim_service_rust_future_free_void(handle)
		},
	)

	return err
}

// Get the session ID
func (_self *FfiSessionContext) SessionId() (uint32, error) {
	_pointer := _self.ffiObject.incrementPointer("*FfiSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) C.uint32_t {
		return C.uniffi_slim_service_fn_method_ffisessioncontext_session_id(
			_pointer, _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue uint32
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterUint32INSTANCE.Lift(_uniffiRV), nil
	}
}

// Get the session type (PointToPoint or Multicast)
func (_self *FfiSessionContext) SessionType() (SessionType, error) {
	_pointer := _self.ffiObject.incrementPointer("*FfiSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_service_fn_method_ffisessioncontext_session_type(
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
func (_self *FfiSessionContext) Source() (Name, error) {
	_pointer := _self.ffiObject.incrementPointer("*FfiSessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_service_fn_method_ffisessioncontext_source(
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
func (object *FfiSessionContext) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterFfiSessionContext struct{}

var FfiConverterFfiSessionContextINSTANCE = FfiConverterFfiSessionContext{}

func (c FfiConverterFfiSessionContext) Lift(pointer unsafe.Pointer) *FfiSessionContext {
	result := &FfiSessionContext{
		newFfiObject(
			pointer,
			func(pointer unsafe.Pointer, status *C.RustCallStatus) unsafe.Pointer {
				return C.uniffi_slim_service_fn_clone_ffisessioncontext(pointer, status)
			},
			func(pointer unsafe.Pointer, status *C.RustCallStatus) {
				C.uniffi_slim_service_fn_free_ffisessioncontext(pointer, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*FfiSessionContext).Destroy)
	return result
}

func (c FfiConverterFfiSessionContext) Read(reader io.Reader) *FfiSessionContext {
	return c.Lift(unsafe.Pointer(uintptr(readUint64(reader))))
}

func (c FfiConverterFfiSessionContext) Lower(value *FfiSessionContext) unsafe.Pointer {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the pointer will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked pointer.
	pointer := value.ffiObject.incrementPointer("*FfiSessionContext")
	defer value.ffiObject.decrementPointer()
	return pointer

}

func (c FfiConverterFfiSessionContext) Write(writer io.Writer, value *FfiSessionContext) {
	writeUint64(writer, uint64(uintptr(c.Lower(value))))
}

type FfiDestroyerFfiSessionContext struct{}

func (_ FfiDestroyerFfiSessionContext) Destroy(value *FfiSessionContext) {
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

// Message context for received messages
type MessageContext struct {
	SourceName      Name
	DestinationName *Name
	PayloadType     string
	Metadata        map[string]string
	InputConnection uint64
	Identity        string
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
	// Session type (PointToPoint or Multicast)
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
	Insecure bool
}

func (r *TlsConfig) Destroy() {
	FfiDestroyerBool{}.Destroy(r.Insecure)
}

type FfiConverterTlsConfig struct{}

var FfiConverterTlsConfigINSTANCE = FfiConverterTlsConfig{}

func (c FfiConverterTlsConfig) Lift(rb RustBufferI) TlsConfig {
	return LiftFromRustBuffer[TlsConfig](c, rb)
}

func (c FfiConverterTlsConfig) Read(reader io.Reader) TlsConfig {
	return TlsConfig{
		FfiConverterBoolINSTANCE.Read(reader),
	}
}

func (c FfiConverterTlsConfig) Lower(value TlsConfig) C.RustBuffer {
	return LowerIntoRustBuffer[TlsConfig](c, value)
}

func (c FfiConverterTlsConfig) Write(writer io.Writer, value TlsConfig) {
	FfiConverterBoolINSTANCE.Write(writer, value.Insecure)
}

type FfiDestroyerTlsConfig struct{}

func (_ FfiDestroyerTlsConfig) Destroy(value TlsConfig) {
	value.Destroy()
}

// Session type enum
type SessionType uint

const (
	SessionTypePointToPoint SessionType = 1
	SessionTypeMulticast    SessionType = 2
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
var ErrSlimErrorConfigError = fmt.Errorf("SlimErrorConfigError")
var ErrSlimErrorSessionError = fmt.Errorf("SlimErrorSessionError")
var ErrSlimErrorReceiveError = fmt.Errorf("SlimErrorReceiveError")
var ErrSlimErrorSendError = fmt.Errorf("SlimErrorSendError")
var ErrSlimErrorAuthError = fmt.Errorf("SlimErrorAuthError")
var ErrSlimErrorTimeout = fmt.Errorf("SlimErrorTimeout")
var ErrSlimErrorInvalidArgument = fmt.Errorf("SlimErrorInvalidArgument")
var ErrSlimErrorInternalError = fmt.Errorf("SlimErrorInternalError")

// Variant structs
type SlimErrorConfigError struct {
	Message string
}

func NewSlimErrorConfigError(
	message string,
) *SlimError {
	return &SlimError{err: &SlimErrorConfigError{
		Message: message}}
}

func (e SlimErrorConfigError) destroy() {
	FfiDestroyerString{}.Destroy(e.Message)
}

func (err SlimErrorConfigError) Error() string {
	return fmt.Sprint("ConfigError",
		": ",

		"Message=",
		err.Message,
	)
}

func (self SlimErrorConfigError) Is(target error) bool {
	return target == ErrSlimErrorConfigError
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
		return &SlimError{&SlimErrorConfigError{
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
	case *SlimErrorConfigError:
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
	case SlimErrorConfigError:
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

//export slim_service_uniffiFutureContinuationCallback
func slim_service_uniffiFutureContinuationCallback(data C.uint64_t, pollResult C.int8_t) {
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
			(C.UniffiRustFutureContinuationCallback)(C.slim_service_uniffiFutureContinuationCallback),
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

//export slim_service_uniffiFreeGorutine
func slim_service_uniffiFreeGorutine(data C.uint64_t) {
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
		return C.uniffi_slim_service_fn_func_create_app_with_secret(FfiConverterNameINSTANCE.Lower(appName), FfiConverterStringINSTANCE.Lower(sharedSecret), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *BindingsAdapter
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterBindingsAdapterINSTANCE.Lift(_uniffiRV), nil
	}
}

// Get the version of the SLIM bindings
func GetVersion() string {
	return FfiConverterStringINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_service_fn_func_get_version(_uniffiStatus),
		}
	}))
}

// Initialize the crypto provider
func InitializeCrypto() {
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_service_fn_func_initialize_crypto(_uniffiStatus)
		return false
	})
}
