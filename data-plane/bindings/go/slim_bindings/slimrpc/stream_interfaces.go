package slimrpc

import (
	"fmt"

	slim_bindings "github.com/agntcy/slim-bindings-go"
	"google.golang.org/protobuf/proto"
)


// ResponseStream is a generic stream for receiving responses
type ResponseStream[T proto.Message] interface {
	Recv() (T, error)
}


// RequestStream is a generic stream for sending requests
type RequestStream[T proto.Message] interface {
	Send(T) error
}


// ClientRequestStream is a generic client stream for sending requests and receiving a final response
type ClientRequestStream[TReq proto.Message, TResp proto.Message] interface {
	Send(TReq) error
	CloseAndRecv() (TResp, error)
}


// ClientBidiStream is a generic client stream for bidirectional streaming
// Send sends requests, Recv receives responses
type ClientBidiStream[TReq proto.Message, TResp proto.Message] interface {
	Send(TReq) error
	Recv() (TResp, error)
	CloseSend() error
}


// ServerBidiStream is a generic server stream for bidirectional streaming
// Send sends responses, Recv receives requests
type ServerBidiStream[TReq proto.Message, TResp proto.Message] interface {
	Send(TResp) error
	Recv() (TReq, error)
}


// Generic client response stream implementation
type genericClientResponseStream[T proto.Message] struct {
	stream *slim_bindings.ResponseStreamReader
}


func NewClientResponseStream[T proto.Message](stream *slim_bindings.ResponseStreamReader) ResponseStream[T] {
	return &genericClientResponseStream[T]{stream: stream}
}


func (s *genericClientResponseStream[T]) Recv() (T, error) {
	var zero T
	msg := s.stream.NextAsync()
	switch m := msg.(type) {
	case slim_bindings.StreamMessageEnd:
		return zero, nil
	case slim_bindings.StreamMessageError:
		return zero, m.Field0.AsError()
	case slim_bindings.StreamMessageData:
		resp := zero.ProtoReflect().New().Interface().(T)
		if err := proto.Unmarshal(m.Field0, resp); err != nil {
			return zero, err
		}
		return resp, nil
	default:
		return zero, fmt.Errorf("unknown stream message type")
	}
}


// Generic client request stream implementation
type genericClientRequestStream[TReq proto.Message, TResp proto.Message] struct {
	stream *slim_bindings.RequestStreamWriter
}


func NewClientRequestStream[TReq proto.Message, TResp proto.Message](stream *slim_bindings.RequestStreamWriter) ClientRequestStream[TReq, TResp] {
	return &genericClientRequestStream[TReq, TResp]{stream: stream}
}


func (s *genericClientRequestStream[TReq, TResp]) Send(req TReq) error {
	reqBytes, err := proto.Marshal(req)
	if err != nil {
		return err
	}
	return s.stream.SendAsync(reqBytes)
}


func (s *genericClientRequestStream[TReq, TResp]) CloseAndRecv() (TResp, error) {
	var zero TResp
	respBytes, err := s.stream.FinalizeAsync()
	if err != nil {
		return zero, err
	}

	resp := zero.ProtoReflect().New().Interface().(TResp)
	if err := proto.Unmarshal(respBytes, resp); err != nil {
		return zero, err
	}
	return resp, nil
}


// Generic client bidi stream implementation
type genericClientBidiStream[TReq proto.Message, TResp proto.Message] struct {
	stream *slim_bindings.BidiStreamHandler
}


func NewClientBidiStream[TReq proto.Message, TResp proto.Message](stream *slim_bindings.BidiStreamHandler) ClientBidiStream[TReq, TResp] {
	return &genericClientBidiStream[TReq, TResp]{stream: stream}
}


func (s *genericClientBidiStream[TReq, TResp]) Send(req TReq) error {
	reqBytes, err := proto.Marshal(req)
	if err != nil {
		return err
	}
	return s.stream.SendAsync(reqBytes)
}


func (s *genericClientBidiStream[TReq, TResp]) Recv() (TResp, error) {
	var zero TResp
	msg := s.stream.RecvAsync()
	switch m := msg.(type) {
	case slim_bindings.StreamMessageEnd:
		return zero, nil
	case slim_bindings.StreamMessageError:
		return zero, m.Field0.AsError()
	case slim_bindings.StreamMessageData:
		resp := zero.ProtoReflect().New().Interface().(TResp)
		if err := proto.Unmarshal(m.Field0, resp); err != nil {
			return zero, err
		}
		return resp, nil
	default:
		return zero, fmt.Errorf("unknown stream message type")
	}
}


func (s *genericClientBidiStream[TReq, TResp]) CloseSend() error {
	return s.stream.CloseSendAsync()
}


// Generic server response stream implementation
type genericServerResponseStream[T proto.Message] struct {
	stream *slim_bindings.RequestStream
}


func NewServerResponseStream[T proto.Message](stream *slim_bindings.RequestStream) ResponseStream[T] {
	return &genericServerResponseStream[T]{stream: stream}
}


func (s *genericServerResponseStream[T]) Recv() (T, error) {
	var zero T
	msg := s.stream.NextAsync()
	switch m := msg.(type) {
	case slim_bindings.StreamMessageEnd:
		return zero, nil
	case slim_bindings.StreamMessageError:
		return zero, m.Field0.AsError()
	case slim_bindings.StreamMessageData:
		req := zero.ProtoReflect().New().Interface().(T)
		if err := proto.Unmarshal(m.Field0, req); err != nil {
			return zero, err
		}
		return req, nil
	default:
		return zero, fmt.Errorf("unknown stream message type")
	}
}


// Generic server request stream implementation
type genericServerRequestStream[T proto.Message] struct {
	sink *slim_bindings.ResponseSink
}


func NewServerRequestStream[T proto.Message](sink *slim_bindings.ResponseSink) RequestStream[T] {
	return &genericServerRequestStream[T]{sink: sink}
}


func (s *genericServerRequestStream[T]) Send(resp T) error {
	respBytes, err := proto.Marshal(resp)
	if err != nil {
		return err
	}
	return s.sink.SendAsync(respBytes)
}


// Generic server bidi stream implementation
type genericServerBidiStream[TReq proto.Message, TResp proto.Message] struct {
	stream *slim_bindings.RequestStream
	sink   *slim_bindings.ResponseSink
}


func NewServerBidiStream[TReq proto.Message, TResp proto.Message](stream *slim_bindings.RequestStream, sink *slim_bindings.ResponseSink) ServerBidiStream[TReq, TResp] {
	return &genericServerBidiStream[TReq, TResp]{stream: stream, sink: sink}
}


func (s *genericServerBidiStream[TReq, TResp]) Send(resp TResp) error {
	respBytes, err := proto.Marshal(resp)
	if err != nil {
		return err
	}
	return s.sink.SendAsync(respBytes)
}


func (s *genericServerBidiStream[TReq, TResp]) Recv() (TReq, error) {
	var zero TReq
	msg := s.stream.NextAsync()
	switch m := msg.(type) {
	case slim_bindings.StreamMessageEnd:
		return zero, nil
	case slim_bindings.StreamMessageError:
		return zero, m.Field0.AsError()
	case slim_bindings.StreamMessageData:
		req := zero.ProtoReflect().New().Interface().(TReq)
		if err := proto.Unmarshal(m.Field0, req); err != nil {
			return zero, err
		}
		return req, nil
	default:
		return zero, fmt.Errorf("unknown stream message type")
	}
}


