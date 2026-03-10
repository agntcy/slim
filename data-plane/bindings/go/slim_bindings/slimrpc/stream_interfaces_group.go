//go:build slim_multicast

// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package slimrpc

import (
	"fmt"

	slim_bindings "github.com/agntcy/slim-bindings-go"
	"google.golang.org/protobuf/proto"
)

// MulticastItem pairs a decoded response with the context of the server that sent it.
type MulticastItem[T any] struct {
	Context slim_bindings.RpcMessageContext
	Value   T
}

// MulticastResponseStream receives decoded responses from multiple group members.
// Recv returns (nil, nil) when the stream ends.
type MulticastResponseStream[T proto.Message] interface {
	Recv() (*MulticastItem[T], error)
}

// MulticastClientBidiStream is a bidirectional group stream.
// Send serializes and sends requests; Recv deserializes and returns per-member responses.
// Recv returns (nil, nil) when the stream ends.
type MulticastClientBidiStream[TReq proto.Message, TResp proto.Message] interface {
	Send(TReq) error
	CloseSend() error
	Recv() (*MulticastItem[TResp], error)
}

// --- generic implementations ---

type genericMulticastResponseStream[T proto.Message] struct {
	reader *slim_bindings.MulticastResponseReader
}

func NewMulticastResponseStream[T proto.Message](reader *slim_bindings.MulticastResponseReader) MulticastResponseStream[T] {
	return &genericMulticastResponseStream[T]{reader: reader}
}

func (s *genericMulticastResponseStream[T]) Recv() (*MulticastItem[T], error) {
	var zero T
	msg := s.reader.NextAsync()
	switch v := msg.(type) {
	case slim_bindings.MulticastStreamMessageEnd:
		_ = v
		return nil, nil
	case slim_bindings.MulticastStreamMessageError:
		return nil, v.Error.AsError()
	case slim_bindings.MulticastStreamMessageData:
		resp := zero.ProtoReflect().New().Interface().(T)
		if err := proto.Unmarshal(v.Item.Message, resp); err != nil {
			return nil, err
		}
		return &MulticastItem[T]{Context: v.Item.Context, Value: resp}, nil
	default:
		return nil, fmt.Errorf("unknown multicast stream message type")
	}
}

type genericMulticastClientBidiStream[TReq proto.Message, TResp proto.Message] struct {
	handler *slim_bindings.MulticastBidiStreamHandler
}

func NewMulticastClientBidiStream[TReq proto.Message, TResp proto.Message](handler *slim_bindings.MulticastBidiStreamHandler) MulticastClientBidiStream[TReq, TResp] {
	return &genericMulticastClientBidiStream[TReq, TResp]{handler: handler}
}

func (s *genericMulticastClientBidiStream[TReq, TResp]) Send(req TReq) error {
	reqBytes, err := proto.Marshal(req)
	if err != nil {
		return err
	}
	return s.handler.SendAsync(reqBytes)
}

func (s *genericMulticastClientBidiStream[TReq, TResp]) CloseSend() error {
	return s.handler.CloseSendAsync()
}

func (s *genericMulticastClientBidiStream[TReq, TResp]) Recv() (*MulticastItem[TResp], error) {
	var zero TResp
	msg := s.handler.RecvAsync()
	switch v := msg.(type) {
	case slim_bindings.MulticastStreamMessageEnd:
		_ = v
		return nil, nil
	case slim_bindings.MulticastStreamMessageError:
		return nil, v.Error.AsError()
	case slim_bindings.MulticastStreamMessageData:
		resp := zero.ProtoReflect().New().Interface().(TResp)
		if err := proto.Unmarshal(v.Item.Message, resp); err != nil {
			return nil, err
		}
		return &MulticastItem[TResp]{Context: v.Item.Context, Value: resp}, nil
	default:
		return nil, fmt.Errorf("unknown multicast stream message type")
	}
}
