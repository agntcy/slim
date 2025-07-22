package nodecontrol

import (
	"reflect"
	"testing"
	"time"

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
