package nodecontrol

import (
	"context"
	"fmt"
	"reflect"
	"strings"
	"sync"
	"testing"
	"time"

	"google.golang.org/grpc/metadata"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
)

func TestWaitForResponseByType_MultipleNodes(t *testing.T) {
	ms := DefaultNodeCommandHandler()

	// Add messages to multiple nodes
	msg1 := &controllerapi.ControlMessage{
		MessageId: "node1-msg",
		Payload: &controllerapi.ControlMessage_ConfigCommand{
			ConfigCommand: &controllerapi.ConfigurationCommand{},
		},
	}

	msg2 := &controllerapi.ControlMessage{
		MessageId: "node2-msg",
		Payload: &controllerapi.ControlMessage_ConfigCommand{
			ConfigCommand: &controllerapi.ConfigurationCommand{},
		},
	}

	// Test that we can wait for and receive messages from specific nodes
	// Start goroutines to simulate ResponseReceived calls
	go func() {
		time.Sleep(100 * time.Millisecond)
		ms.ResponseReceived("node1", msg1)
	}()

	go func() {
		time.Sleep(200 * time.Millisecond)
		ms.ResponseReceived("node2", msg2)
	}()

	// Wait for message from node1 specifically
	foundMsg, err := ms.WaitForResponse("node1", reflect.TypeOf(&controllerapi.ControlMessage_ConfigCommand{}))
	if err != nil {
		t.Fatalf("unexpected error waiting for node1 message: %v", err)
	}

	if foundMsg.MessageId != "node1-msg" {
		t.Errorf("expected node1-msg, got %s", foundMsg.MessageId)
	}

	// Wait for message from node2 specifically
	foundMsg, err = ms.WaitForResponse("node2", reflect.TypeOf(&controllerapi.ControlMessage_ConfigCommand{}))
	if err != nil {
		t.Fatalf("unexpected error waiting for node2 message: %v", err)
	}

	if foundMsg.MessageId != "node2-msg" {
		t.Errorf("expected node2-msg, got %s", foundMsg.MessageId)
	}
}

func TestWaitForResponseByType_EmptyNodeID(t *testing.T) {
	ms := DefaultNodeCommandHandler()

	_, err := ms.WaitForResponse("", reflect.TypeOf(&controllerapi.ControlMessage_ConfigCommand{}))
	if err == nil {
		t.Fatal("expected error when nodeID is empty")
	}
}

func TestWaitForResponseByType_NilMessageType(t *testing.T) {
	ms := DefaultNodeCommandHandler()

	_, err := ms.WaitForResponse("node1", nil)
	if err == nil {
		t.Fatal("expected error when messageType is nil")
	}
}

func TestWaitForResponseByType_Timeout(t *testing.T) {
	ms := DefaultNodeCommandHandler()

	// This should timeout since no message will be received
	start := time.Now()
	_, err := ms.WaitForResponse("node1", reflect.TypeOf(&controllerapi.ControlMessage_ConfigCommand{}))
	duration := time.Since(start)

	if err == nil {
		t.Fatal("expected timeout error")
	}

	// Should timeout after approximately 10 seconds
	if duration < 9*time.Second || duration > 11*time.Second {
		t.Errorf("expected timeout around 10 seconds, got %v", duration)
	}
}

func TestSendMessage_NodeIDEmpty(t *testing.T) {
	ms := DefaultNodeCommandHandler()
	err := ms.SendMessage("", &controllerapi.ControlMessage{MessageId: "msg1"})
	if err == nil || err.Error() != "nodeID cannot be empty" {
		t.Fatalf("expected error for empty nodeID, got: %v", err)
	}
}

type mockStream struct {
	sentMessages []*controllerapi.ControlMessage
	sendMu       sync.Mutex
	sendDelay    time.Duration
	failSend     bool
}

func (m *mockStream) Send(msg *controllerapi.ControlMessage) error {
	m.sendMu.Lock()
	defer m.sendMu.Unlock()
	if m.failSend {
		return fmt.Errorf("send failed")
	}
	if m.sendDelay > 0 {
		time.Sleep(m.sendDelay)
	}
	m.sentMessages = append(m.sentMessages, msg)
	return nil
}

func (m *mockStream) Recv() (*controllerapi.ControlMessage, error) {
	return nil, nil
}

func (m *mockStream) SetHeader(metadata.MD) error  { return nil }
func (m *mockStream) SendHeader(metadata.MD) error { return nil }
func (m *mockStream) SetTrailer(metadata.MD)       {}
func (m *mockStream) Context() context.Context     { return context.Background() }
func (m *mockStream) SendMsg(interface{}) error    { return nil }
func (m *mockStream) RecvMsg(interface{}) error    { return nil }

func TestSendMessage_NodeNotConnected(t *testing.T) {
	ms := DefaultNodeCommandHandler()
	nodeID := "node1"
	// No connection status set, so should error
	err := ms.SendMessage(nodeID, &controllerapi.ControlMessage{MessageId: "msg1"})
	if err == nil || !contains(err.Error(), "failed to get connection status") {
		t.Fatalf("expected error for missing connection status, got: %v", err)
	}
}

func TestSendMessage_NodeNotConnectedStatus(t *testing.T) {
	ms := DefaultNodeCommandHandler()
	nodeID := "node1"
	ms.UpdateConnectionStatus(nodeID, NodeStatusNotConnected)
	err := ms.SendMessage(nodeID, &controllerapi.ControlMessage{MessageId: "msg1"})
	if err == nil || !contains(err.Error(), "is not connected") {
		t.Fatalf("expected error for not connected node, got: %v", err)
	}
}

func TestSendMessage_NoStream(t *testing.T) {
	ms := DefaultNodeCommandHandler()
	nodeID := "node1"
	ms.UpdateConnectionStatus(nodeID, NodeStatusConnected)
	// No stream added
	err := ms.SendMessage(nodeID, &controllerapi.ControlMessage{MessageId: "msg1"})
	if err == nil || !contains(err.Error(), "no stream found") {
		t.Fatalf("expected error for missing stream, got: %v", err)
	}
}

func TestSendMessage_SendFails(t *testing.T) {
	ms := DefaultNodeCommandHandler()
	nodeID := "node1"
	ms.UpdateConnectionStatus(nodeID, NodeStatusConnected)
	mock := &mockStream{failSend: true}
	ms.AddStream(nodeID, mock)
	err := ms.SendMessage(nodeID, &controllerapi.ControlMessage{MessageId: "msg1"})
	if err == nil || !contains(err.Error(), "failed to send message") {
		t.Fatalf("expected error for send failure, got: %v", err)
	}
}

func TestSendMessage_Success(t *testing.T) {
	ms := DefaultNodeCommandHandler()
	nodeID := "node1"
	ms.UpdateConnectionStatus(nodeID, NodeStatusConnected)
	mock := &mockStream{}
	ms.AddStream(nodeID, mock)
	msg := &controllerapi.ControlMessage{MessageId: "msg1"}
	err := ms.SendMessage(nodeID, msg)
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if len(mock.sentMessages) != 1 || mock.sentMessages[0] != msg {
		t.Fatalf("expected message to be sent")
	}
}

func TestSendMessage_SerializesPerNodeID(t *testing.T) {
	ms := DefaultNodeCommandHandler()
	nodeID := "node1"
	ms.UpdateConnectionStatus(nodeID, NodeStatusConnected)
	mock := &mockStream{sendDelay: 200 * time.Millisecond}
	ms.AddStream(nodeID, mock)

	var wg sync.WaitGroup
	start := time.Now()
	wg.Add(2)
	go func() {
		defer wg.Done()
		_ = ms.SendMessage(nodeID, &controllerapi.ControlMessage{MessageId: "msg1"})
	}()
	go func() {
		defer wg.Done()
		_ = ms.SendMessage(nodeID, &controllerapi.ControlMessage{MessageId: "msg2"})
	}()
	wg.Wait()
	elapsed := time.Since(start)
	if elapsed < 400*time.Millisecond {
		t.Errorf("expected serialization, got elapsed=%v", elapsed)
	}
	if len(mock.sentMessages) != 2 {
		t.Errorf("expected 2 messages sent, got %d", len(mock.sentMessages))
	}
}

func contains(s, substr string) bool {
	return strings.Contains(s, substr)
}
