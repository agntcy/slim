package nodecontrol

import (
	"reflect"
	"testing"

	"github.com/google/uuid"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
)

func TestFindMessageByType(t *testing.T) {
	ms := DefaultNodeCommandHandler()

	// Add some test messages
	configMsg := &controllerapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload: &controllerapi.ControlMessage_ConfigCommand{
			ConfigCommand: &controllerapi.ConfigurationCommand{
				ConnectionsToCreate: []*controllerapi.Connection{{
					ConnectionId: "c1",
					ConfigData:   "{\"endpoint\":\"10.0.0.1:8080\"}",
				}},
			},
		},
	}

	subscriptionMsg := &controllerapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload: &controllerapi.ControlMessage_SubscriptionListRequest{
			SubscriptionListRequest: &controllerapi.SubscriptionListRequest{},
		},
	}

	ackMsg := &controllerapi.ControlMessage{
		MessageId: uuid.NewString(),
		Payload: &controllerapi.ControlMessage_Ack{
			Ack: &controllerapi.Ack{
				OriginalMessageId: "test-123",
				Success:           true,
			},
		},
	}

	// Add messages to different nodes
	ms.ResponseReceived("node1", configMsg)
	ms.ResponseReceived("node2", subscriptionMsg)
	ms.ResponseReceived("node1", ackMsg)

	tests := []struct {
		name          string
		messageType   reflect.Type
		expectedFound bool
		expectedMsg   *controllerapi.ControlMessage
	}{
		{
			name:          "find config command",
			messageType:   reflect.TypeOf(&controllerapi.ControlMessage_ConfigCommand{}),
			expectedFound: true,
			expectedMsg:   configMsg,
		},
		{
			name:          "find subscription list request",
			messageType:   reflect.TypeOf(&controllerapi.ControlMessage_SubscriptionListRequest{}),
			expectedFound: true,
			expectedMsg:   subscriptionMsg,
		},
		{
			name:          "find ack message",
			messageType:   reflect.TypeOf(&controllerapi.ControlMessage_Ack{}),
			expectedFound: true,
			expectedMsg:   ackMsg,
		},
		{
			name:          "message type not found",
			messageType:   reflect.TypeOf(&controllerapi.ControlMessage_ConnectionListRequest{}),
			expectedFound: false,
			expectedMsg:   nil,
		},
	}

	for _, tt := range tests {
		t.Run(tt.name, func(t *testing.T) {
			foundMsg, err := ms.WaitForResponse(tt.messageType)

			if tt.expectedFound {
				if err != nil {
					t.Fatalf("expected to find message, got error: %v", err)
				}
				if foundMsg == nil {
					t.Fatal("expected non-nil message")
				}
				if foundMsg.MessageId != tt.expectedMsg.MessageId {
					t.Errorf("expected message ID %s, got %s", tt.expectedMsg.MessageId, foundMsg.MessageId)
				}
			} else {
				if err == nil {
					t.Fatal("expected error for message type not found")
				}
				if foundMsg != nil {
					t.Errorf("expected nil message, got %+v", foundMsg)
				}
			}
		})
	}
}

func TestFindMessageByType_RemovesMessage(t *testing.T) {
	ms := DefaultNodeCommandHandler()

	// Add two messages of the same type to the same node
	msg1 := &controllerapi.ControlMessage{
		MessageId: "msg1",
		Payload: &controllerapi.ControlMessage_ConfigCommand{
			ConfigCommand: &controllerapi.ConfigurationCommand{},
		},
	}

	msg2 := &controllerapi.ControlMessage{
		MessageId: "msg2",
		Payload: &controllerapi.ControlMessage_ConfigCommand{
			ConfigCommand: &controllerapi.ConfigurationCommand{},
		},
	}

	ms.ResponseReceived("node1", msg1)
	ms.ResponseReceived("node1", msg2)

	// Find first message - should return msg1 and remove it
	foundMsg, err := ms.WaitForResponse(reflect.TypeOf(&controllerapi.ControlMessage_ConfigCommand{}))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if foundMsg.MessageId != "msg1" {
		t.Errorf("expected first message (msg1), got %s", foundMsg.MessageId)
	}

	// Find second message - should return msg2 and remove it
	foundMsg, err = ms.WaitForResponse(reflect.TypeOf(&controllerapi.ControlMessage_ConfigCommand{}))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}
	if foundMsg.MessageId != "msg2" {
		t.Errorf("expected second message (msg2), got %s", foundMsg.MessageId)
	}

	// Try to find third message - should fail
	_, err = ms.WaitForResponse(reflect.TypeOf(&controllerapi.ControlMessage_ConfigCommand{}))
	if err == nil {
		t.Fatal("expected error when no more messages available")
	}
}

func TestFindMessageByType_EmptyMap(t *testing.T) {
	ms := DefaultNodeCommandHandler()

	_, err := ms.WaitForResponse(reflect.TypeOf(&controllerapi.ControlMessage_ConfigCommand{}))
	if err == nil {
		t.Fatal("expected error when searching in empty map")
	}
}

func TestFindMessageByType_MultipleNodes(t *testing.T) {
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

	ms.ResponseReceived("node1", msg1)
	ms.ResponseReceived("node2", msg2)

	// Should find one of the messages (order not guaranteed due to sync.Map.Range)
	foundMsg, err := ms.WaitForResponse(reflect.TypeOf(&controllerapi.ControlMessage_ConfigCommand{}))
	if err != nil {
		t.Fatalf("unexpected error: %v", err)
	}

	if foundMsg.MessageId != "node1-msg" && foundMsg.MessageId != "node2-msg" {
		t.Errorf("expected either node1-msg or node2-msg, got %s", foundMsg.MessageId)
	}
}
