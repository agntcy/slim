package slim_bindings

// #include <slimbindings.h>
import "C"

import (
	"bytes"
	"encoding/binary"
	"fmt"
	"io"
	"math"
	"runtime"
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
		C.ffi_slim_go_bindings_rustbuffer_free(cb.inner, status)
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
		return C.ffi_slim_go_bindings_rustbuffer_from_bytes(foreign, status)
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
		return C.ffi_slim_go_bindings_uniffi_contract_version()
	})
	if bindingsContractVersion != int(scaffoldingContractVersion) {
		// If this happens try cleaning and rebuilding your project
		panic("slim_bindings: UniFFI contract version mismatch")
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_func_get_version()
		})
		if checksum != 17765 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_func_get_version: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_func_initialize_crypto()
		})
		if checksum != 43273 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_func_initialize_crypto: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_app_create_session()
		})
		if checksum != 22321 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_app_create_session: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_app_delete_session()
		})
		if checksum != 48463 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_app_delete_session: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_app_id()
		})
		if checksum != 26200 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_app_id: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_app_listen_for_session()
		})
		if checksum != 17329 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_app_listen_for_session: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_app_name()
		})
		if checksum != 50181 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_app_name: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_app_remove_route()
		})
		if checksum != 12403 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_app_remove_route: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_app_set_route()
		})
		if checksum != 24135 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_app_set_route: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_app_subscribe()
		})
		if checksum != 6892 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_app_subscribe: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_app_unsubscribe()
		})
		if checksum != 7256 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_app_unsubscribe: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_service_create_app()
		})
		if checksum != 20426 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_service_create_app: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_sessioncontext_get_message()
		})
		if checksum != 64930 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_sessioncontext_get_message: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_sessioncontext_invite()
		})
		if checksum != 29148 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_sessioncontext_invite: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_sessioncontext_publish()
		})
		if checksum != 49064 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_sessioncontext_publish: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_sessioncontext_publish_to()
		})
		if checksum != 23168 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_sessioncontext_publish_to: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_method_sessioncontext_remove()
		})
		if checksum != 42961 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_method_sessioncontext_remove: UniFFI API checksum mismatch")
		}
	}
	{
		checksum := rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint16_t {
			return C.uniffi_slim_go_bindings_checksum_constructor_service_new()
		})
		if checksum != 1659 {
			// If this happens try cleaning and rebuilding your project
			panic("slim_bindings: uniffi_slim_go_bindings_checksum_constructor_service_new: UniFFI API checksum mismatch")
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

type AppInterface interface {
	CreateSession(config SessionConfig, destination Name) (*SessionContext, error)
	DeleteSession(session *SessionContext) error
	Id() uint64
	ListenForSession(timeoutMs *uint32) (*SessionContext, error)
	Name() Name
	RemoveRoute(name Name, connectionId uint64) error
	SetRoute(name Name, connectionId uint64) error
	Subscribe(name Name, connectionId *uint64) error
	Unsubscribe(name Name, connectionId *uint64) error
}
type App struct {
	ffiObject FfiObject
}

func (_self *App) CreateSession(config SessionConfig, destination Name) (*SessionContext, error) {
	_pointer := _self.ffiObject.incrementPointer("*App")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_go_bindings_fn_method_app_create_session(
			_pointer, FfiConverterSessionConfigINSTANCE.Lower(config), FfiConverterNameINSTANCE.Lower(destination), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *SessionContext
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterSessionContextINSTANCE.Lift(_uniffiRV), nil
	}
}

func (_self *App) DeleteSession(session *SessionContext) error {
	_pointer := _self.ffiObject.incrementPointer("*App")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_go_bindings_fn_method_app_delete_session(
			_pointer, FfiConverterSessionContextINSTANCE.Lower(session), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

func (_self *App) Id() uint64 {
	_pointer := _self.ffiObject.incrementPointer("*App")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterUint64INSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) C.uint64_t {
		return C.uniffi_slim_go_bindings_fn_method_app_id(
			_pointer, _uniffiStatus)
	}))
}

func (_self *App) ListenForSession(timeoutMs *uint32) (*SessionContext, error) {
	_pointer := _self.ffiObject.incrementPointer("*App")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_go_bindings_fn_method_app_listen_for_session(
			_pointer, FfiConverterOptionalUint32INSTANCE.Lower(timeoutMs), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *SessionContext
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterSessionContextINSTANCE.Lift(_uniffiRV), nil
	}
}

func (_self *App) Name() Name {
	_pointer := _self.ffiObject.incrementPointer("*App")
	defer _self.ffiObject.decrementPointer()
	return FfiConverterNameINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_go_bindings_fn_method_app_name(
				_pointer, _uniffiStatus),
		}
	}))
}

func (_self *App) RemoveRoute(name Name, connectionId uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*App")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_go_bindings_fn_method_app_remove_route(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterUint64INSTANCE.Lower(connectionId), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

func (_self *App) SetRoute(name Name, connectionId uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*App")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_go_bindings_fn_method_app_set_route(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterUint64INSTANCE.Lower(connectionId), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

func (_self *App) Subscribe(name Name, connectionId *uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*App")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_go_bindings_fn_method_app_subscribe(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterOptionalUint64INSTANCE.Lower(connectionId), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

func (_self *App) Unsubscribe(name Name, connectionId *uint64) error {
	_pointer := _self.ffiObject.incrementPointer("*App")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_go_bindings_fn_method_app_unsubscribe(
			_pointer, FfiConverterNameINSTANCE.Lower(name), FfiConverterOptionalUint64INSTANCE.Lower(connectionId), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}
func (object *App) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterApp struct{}

var FfiConverterAppINSTANCE = FfiConverterApp{}

func (c FfiConverterApp) Lift(pointer unsafe.Pointer) *App {
	result := &App{
		newFfiObject(
			pointer,
			func(pointer unsafe.Pointer, status *C.RustCallStatus) unsafe.Pointer {
				return C.uniffi_slim_go_bindings_fn_clone_app(pointer, status)
			},
			func(pointer unsafe.Pointer, status *C.RustCallStatus) {
				C.uniffi_slim_go_bindings_fn_free_app(pointer, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*App).Destroy)
	return result
}

func (c FfiConverterApp) Read(reader io.Reader) *App {
	return c.Lift(unsafe.Pointer(uintptr(readUint64(reader))))
}

func (c FfiConverterApp) Lower(value *App) unsafe.Pointer {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the pointer will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked pointer.
	pointer := value.ffiObject.incrementPointer("*App")
	defer value.ffiObject.decrementPointer()
	return pointer

}

func (c FfiConverterApp) Write(writer io.Writer, value *App) {
	writeUint64(writer, uint64(uintptr(c.Lower(value))))
}

type FfiDestroyerApp struct{}

func (_ FfiDestroyerApp) Destroy(value *App) {
	value.Destroy()
}

type ServiceInterface interface {
	CreateApp(appName Name, sharedSecret string) (*App, error)
}
type Service struct {
	ffiObject FfiObject
}

func NewService() (*Service, error) {
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_go_bindings_fn_constructor_service_new(_uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *Service
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterServiceINSTANCE.Lift(_uniffiRV), nil
	}
}

func (_self *Service) CreateApp(appName Name, sharedSecret string) (*App, error) {
	_pointer := _self.ffiObject.incrementPointer("*Service")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) unsafe.Pointer {
		return C.uniffi_slim_go_bindings_fn_method_service_create_app(
			_pointer, FfiConverterNameINSTANCE.Lower(appName), FfiConverterStringINSTANCE.Lower(sharedSecret), _uniffiStatus)
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue *App
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterAppINSTANCE.Lift(_uniffiRV), nil
	}
}
func (object *Service) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterService struct{}

var FfiConverterServiceINSTANCE = FfiConverterService{}

func (c FfiConverterService) Lift(pointer unsafe.Pointer) *Service {
	result := &Service{
		newFfiObject(
			pointer,
			func(pointer unsafe.Pointer, status *C.RustCallStatus) unsafe.Pointer {
				return C.uniffi_slim_go_bindings_fn_clone_service(pointer, status)
			},
			func(pointer unsafe.Pointer, status *C.RustCallStatus) {
				C.uniffi_slim_go_bindings_fn_free_service(pointer, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*Service).Destroy)
	return result
}

func (c FfiConverterService) Read(reader io.Reader) *Service {
	return c.Lift(unsafe.Pointer(uintptr(readUint64(reader))))
}

func (c FfiConverterService) Lower(value *Service) unsafe.Pointer {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the pointer will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked pointer.
	pointer := value.ffiObject.incrementPointer("*Service")
	defer value.ffiObject.decrementPointer()
	return pointer

}

func (c FfiConverterService) Write(writer io.Writer, value *Service) {
	writeUint64(writer, uint64(uintptr(c.Lower(value))))
}

type FfiDestroyerService struct{}

func (_ FfiDestroyerService) Destroy(value *Service) {
	value.Destroy()
}

type SessionContextInterface interface {
	GetMessage(timeoutMs *uint32) (MessageWithContext, error)
	Invite(peer Name) error
	Publish(destination Name, fanout uint32, payload []byte, connectionId *uint64, payloadType *string, metadata *map[string]string) error
	PublishTo(originalContext MessageContext, payload []byte, payloadType *string, metadata *map[string]string) error
	Remove(peer Name) error
}
type SessionContext struct {
	ffiObject FfiObject
}

func (_self *SessionContext) GetMessage(timeoutMs *uint32) (MessageWithContext, error) {
	_pointer := _self.ffiObject.incrementPointer("*SessionContext")
	defer _self.ffiObject.decrementPointer()
	_uniffiRV, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_go_bindings_fn_method_sessioncontext_get_message(
				_pointer, FfiConverterOptionalUint32INSTANCE.Lower(timeoutMs), _uniffiStatus),
		}
	})
	if _uniffiErr != nil {
		var _uniffiDefaultValue MessageWithContext
		return _uniffiDefaultValue, _uniffiErr
	} else {
		return FfiConverterMessageWithContextINSTANCE.Lift(_uniffiRV), nil
	}
}

func (_self *SessionContext) Invite(peer Name) error {
	_pointer := _self.ffiObject.incrementPointer("*SessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_go_bindings_fn_method_sessioncontext_invite(
			_pointer, FfiConverterNameINSTANCE.Lower(peer), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

func (_self *SessionContext) Publish(destination Name, fanout uint32, payload []byte, connectionId *uint64, payloadType *string, metadata *map[string]string) error {
	_pointer := _self.ffiObject.incrementPointer("*SessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_go_bindings_fn_method_sessioncontext_publish(
			_pointer, FfiConverterNameINSTANCE.Lower(destination), FfiConverterUint32INSTANCE.Lower(fanout), FfiConverterBytesINSTANCE.Lower(payload), FfiConverterOptionalUint64INSTANCE.Lower(connectionId), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

func (_self *SessionContext) PublishTo(originalContext MessageContext, payload []byte, payloadType *string, metadata *map[string]string) error {
	_pointer := _self.ffiObject.incrementPointer("*SessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_go_bindings_fn_method_sessioncontext_publish_to(
			_pointer, FfiConverterMessageContextINSTANCE.Lower(originalContext), FfiConverterBytesINSTANCE.Lower(payload), FfiConverterOptionalStringINSTANCE.Lower(payloadType), FfiConverterOptionalMapStringStringINSTANCE.Lower(metadata), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}

func (_self *SessionContext) Remove(peer Name) error {
	_pointer := _self.ffiObject.incrementPointer("*SessionContext")
	defer _self.ffiObject.decrementPointer()
	_, _uniffiErr := rustCallWithError[SlimError](FfiConverterSlimError{}, func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_go_bindings_fn_method_sessioncontext_remove(
			_pointer, FfiConverterNameINSTANCE.Lower(peer), _uniffiStatus)
		return false
	})
	return _uniffiErr.AsError()
}
func (object *SessionContext) Destroy() {
	runtime.SetFinalizer(object, nil)
	object.ffiObject.destroy()
}

type FfiConverterSessionContext struct{}

var FfiConverterSessionContextINSTANCE = FfiConverterSessionContext{}

func (c FfiConverterSessionContext) Lift(pointer unsafe.Pointer) *SessionContext {
	result := &SessionContext{
		newFfiObject(
			pointer,
			func(pointer unsafe.Pointer, status *C.RustCallStatus) unsafe.Pointer {
				return C.uniffi_slim_go_bindings_fn_clone_sessioncontext(pointer, status)
			},
			func(pointer unsafe.Pointer, status *C.RustCallStatus) {
				C.uniffi_slim_go_bindings_fn_free_sessioncontext(pointer, status)
			},
		),
	}
	runtime.SetFinalizer(result, (*SessionContext).Destroy)
	return result
}

func (c FfiConverterSessionContext) Read(reader io.Reader) *SessionContext {
	return c.Lift(unsafe.Pointer(uintptr(readUint64(reader))))
}

func (c FfiConverterSessionContext) Lower(value *SessionContext) unsafe.Pointer {
	// TODO: this is bad - all synchronization from ObjectRuntime.go is discarded here,
	// because the pointer will be decremented immediately after this function returns,
	// and someone will be left holding onto a non-locked pointer.
	pointer := value.ffiObject.incrementPointer("*SessionContext")
	defer value.ffiObject.decrementPointer()
	return pointer

}

func (c FfiConverterSessionContext) Write(writer io.Writer, value *SessionContext) {
	writeUint64(writer, uint64(uintptr(c.Lower(value))))
}

type FfiDestroyerSessionContext struct{}

func (_ FfiDestroyerSessionContext) Destroy(value *SessionContext) {
	value.Destroy()
}

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

type MessageWithContext struct {
	Context MessageContext
	Payload []byte
}

func (r *MessageWithContext) Destroy() {
	FfiDestroyerMessageContext{}.Destroy(r.Context)
	FfiDestroyerBytes{}.Destroy(r.Payload)
}

type FfiConverterMessageWithContext struct{}

var FfiConverterMessageWithContextINSTANCE = FfiConverterMessageWithContext{}

func (c FfiConverterMessageWithContext) Lift(rb RustBufferI) MessageWithContext {
	return LiftFromRustBuffer[MessageWithContext](c, rb)
}

func (c FfiConverterMessageWithContext) Read(reader io.Reader) MessageWithContext {
	return MessageWithContext{
		FfiConverterMessageContextINSTANCE.Read(reader),
		FfiConverterBytesINSTANCE.Read(reader),
	}
}

func (c FfiConverterMessageWithContext) Lower(value MessageWithContext) C.RustBuffer {
	return LowerIntoRustBuffer[MessageWithContext](c, value)
}

func (c FfiConverterMessageWithContext) Write(writer io.Writer, value MessageWithContext) {
	FfiConverterMessageContextINSTANCE.Write(writer, value.Context)
	FfiConverterBytesINSTANCE.Write(writer, value.Payload)
}

type FfiDestroyerMessageWithContext struct{}

func (_ FfiDestroyerMessageWithContext) Destroy(value MessageWithContext) {
	value.Destroy()
}

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

type SessionConfig struct {
	SessionType SessionType
	EnableMls   bool
}

func (r *SessionConfig) Destroy() {
	FfiDestroyerSessionType{}.Destroy(r.SessionType)
	FfiDestroyerBool{}.Destroy(r.EnableMls)
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
	}
}

func (c FfiConverterSessionConfig) Lower(value SessionConfig) C.RustBuffer {
	return LowerIntoRustBuffer[SessionConfig](c, value)
}

func (c FfiConverterSessionConfig) Write(writer io.Writer, value SessionConfig) {
	FfiConverterSessionTypeINSTANCE.Write(writer, value.SessionType)
	FfiConverterBoolINSTANCE.Write(writer, value.EnableMls)
}

type FfiDestroyerSessionConfig struct{}

func (_ FfiDestroyerSessionConfig) Destroy(value SessionConfig) {
	value.Destroy()
}

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
	message string
}

func NewSlimErrorConfigError() *SlimError {
	return &SlimError{err: &SlimErrorConfigError{}}
}

func (e SlimErrorConfigError) destroy() {
}

func (err SlimErrorConfigError) Error() string {
	return fmt.Sprintf("ConfigError: %s", err.message)
}

func (self SlimErrorConfigError) Is(target error) bool {
	return target == ErrSlimErrorConfigError
}

type SlimErrorSessionError struct {
	message string
}

func NewSlimErrorSessionError() *SlimError {
	return &SlimError{err: &SlimErrorSessionError{}}
}

func (e SlimErrorSessionError) destroy() {
}

func (err SlimErrorSessionError) Error() string {
	return fmt.Sprintf("SessionError: %s", err.message)
}

func (self SlimErrorSessionError) Is(target error) bool {
	return target == ErrSlimErrorSessionError
}

type SlimErrorReceiveError struct {
	message string
}

func NewSlimErrorReceiveError() *SlimError {
	return &SlimError{err: &SlimErrorReceiveError{}}
}

func (e SlimErrorReceiveError) destroy() {
}

func (err SlimErrorReceiveError) Error() string {
	return fmt.Sprintf("ReceiveError: %s", err.message)
}

func (self SlimErrorReceiveError) Is(target error) bool {
	return target == ErrSlimErrorReceiveError
}

type SlimErrorSendError struct {
	message string
}

func NewSlimErrorSendError() *SlimError {
	return &SlimError{err: &SlimErrorSendError{}}
}

func (e SlimErrorSendError) destroy() {
}

func (err SlimErrorSendError) Error() string {
	return fmt.Sprintf("SendError: %s", err.message)
}

func (self SlimErrorSendError) Is(target error) bool {
	return target == ErrSlimErrorSendError
}

type SlimErrorAuthError struct {
	message string
}

func NewSlimErrorAuthError() *SlimError {
	return &SlimError{err: &SlimErrorAuthError{}}
}

func (e SlimErrorAuthError) destroy() {
}

func (err SlimErrorAuthError) Error() string {
	return fmt.Sprintf("AuthError: %s", err.message)
}

func (self SlimErrorAuthError) Is(target error) bool {
	return target == ErrSlimErrorAuthError
}

type SlimErrorTimeout struct {
	message string
}

func NewSlimErrorTimeout() *SlimError {
	return &SlimError{err: &SlimErrorTimeout{}}
}

func (e SlimErrorTimeout) destroy() {
}

func (err SlimErrorTimeout) Error() string {
	return fmt.Sprintf("Timeout: %s", err.message)
}

func (self SlimErrorTimeout) Is(target error) bool {
	return target == ErrSlimErrorTimeout
}

type SlimErrorInvalidArgument struct {
	message string
}

func NewSlimErrorInvalidArgument() *SlimError {
	return &SlimError{err: &SlimErrorInvalidArgument{}}
}

func (e SlimErrorInvalidArgument) destroy() {
}

func (err SlimErrorInvalidArgument) Error() string {
	return fmt.Sprintf("InvalidArgument: %s", err.message)
}

func (self SlimErrorInvalidArgument) Is(target error) bool {
	return target == ErrSlimErrorInvalidArgument
}

type SlimErrorInternalError struct {
	message string
}

func NewSlimErrorInternalError() *SlimError {
	return &SlimError{err: &SlimErrorInternalError{}}
}

func (e SlimErrorInternalError) destroy() {
}

func (err SlimErrorInternalError) Error() string {
	return fmt.Sprintf("InternalError: %s", err.message)
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

	message := FfiConverterStringINSTANCE.Read(reader)
	switch errorID {
	case 1:
		return &SlimError{&SlimErrorConfigError{message}}
	case 2:
		return &SlimError{&SlimErrorSessionError{message}}
	case 3:
		return &SlimError{&SlimErrorReceiveError{message}}
	case 4:
		return &SlimError{&SlimErrorSendError{message}}
	case 5:
		return &SlimError{&SlimErrorAuthError{message}}
	case 6:
		return &SlimError{&SlimErrorTimeout{message}}
	case 7:
		return &SlimError{&SlimErrorInvalidArgument{message}}
	case 8:
		return &SlimError{&SlimErrorInternalError{message}}
	default:
		panic(fmt.Sprintf("Unknown error code %d in FfiConverterSlimError.Read()", errorID))
	}

}

func (c FfiConverterSlimError) Write(writer io.Writer, value *SlimError) {
	switch variantValue := value.err.(type) {
	case *SlimErrorConfigError:
		writeInt32(writer, 1)
	case *SlimErrorSessionError:
		writeInt32(writer, 2)
	case *SlimErrorReceiveError:
		writeInt32(writer, 3)
	case *SlimErrorSendError:
		writeInt32(writer, 4)
	case *SlimErrorAuthError:
		writeInt32(writer, 5)
	case *SlimErrorTimeout:
		writeInt32(writer, 6)
	case *SlimErrorInvalidArgument:
		writeInt32(writer, 7)
	case *SlimErrorInternalError:
		writeInt32(writer, 8)
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

func GetVersion() string {
	return FfiConverterStringINSTANCE.Lift(rustCall(func(_uniffiStatus *C.RustCallStatus) RustBufferI {
		return GoRustBuffer{
			inner: C.uniffi_slim_go_bindings_fn_func_get_version(_uniffiStatus),
		}
	}))
}

func InitializeCrypto() {
	rustCall(func(_uniffiStatus *C.RustCallStatus) bool {
		C.uniffi_slim_go_bindings_fn_func_initialize_crypto(_uniffiStatus)
		return false
	})
}
