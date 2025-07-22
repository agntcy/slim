package messagingservice

import (
	"fmt"
	"reflect"
	"sync"
	"time"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
)

const (
	NodeStatusConnected    NodeStatus = "connected"
	NodeStatusNotConnected NodeStatus = "not_connected"
	NodeStatusUnknown      NodeStatus = "unknown"
)

// Node Status enumeration
type NodeStatus string

type Messaging interface {
	SendMessage(nodeID string, configurationCommand *controllerapi.ControlMessage) error
	AddStream(nodeID string, stream controllerapi.ControllerService_OpenControlChannelServer)
	RemoveStream(nodeID string) error
	GetConnectionStatus(nodeID string) (NodeStatus, error)
	UpdateConnectionStatus(nodeID string, status NodeStatus)

	GetNodeId(stream controllerapi.ControllerService_OpenControlChannelServer) (string, error)

	FindMessageByType(messageType reflect.Type) (*controllerapi.ControlMessage, error)
	AddNodeCommand(nodeID string, command *controllerapi.ControlMessage)
}

type messagingService struct {
	nodeStreamMap           sync.Map // Maps node IDs and streams map[nodeID]controllerapi.ControllerService_OpenControlChannelServer
	nodeConnectionStatusMap sync.Map // Maps node ID to its connection status

	nodeCommandMap sync.Map // Maps node IDs to their command channels map[nodeID][] *controllerapi.ControlMessage

}

// GetNodeId implements Messaging.
func (m *messagingService) GetNodeId(stream controllerapi.ControllerService_OpenControlChannelServer) (string, error) {
	nodeID, ok := m.nodeStreamMap.Load(stream)
	if !ok {
		return "", fmt.Errorf("no node ID found for the provided stream")
	}
	return nodeID.(string), nil
}

// AddNodeCommand implements Messaging.
func (m *messagingService) AddNodeCommand(nodeID string, command *controllerapi.ControlMessage) {
	// Get the existing command list for the node
	commands, _ := m.nodeCommandMap.LoadOrStore(nodeID, []*controllerapi.ControlMessage{})

	// Type assert to a slice of ControlMessage
	commandList := commands.([]*controllerapi.ControlMessage)

	// Append the new command to the list
	commandList = append(commandList, command)

	// Store the updated command list back in the map
	m.nodeCommandMap.Store(nodeID, commandList)
}

// FindMessageByType implements Messaging.
func (m *messagingService) FindMessageByType(messageType reflect.Type) (*controllerapi.ControlMessage, error) {
	// Go through the command map to find the message
	var foundMessage *controllerapi.ControlMessage
	timeout := time.After(10 * time.Second)          // 10 second timeout
	ticker := time.NewTicker(100 * time.Millisecond) // Check every 100ms
	defer ticker.Stop()

	fmt.Println("Waiting for message of type:", messageType)

	for {
		select {
		case <-timeout:
			return nil, fmt.Errorf("timeout waiting for message of type %v", messageType)
		case <-ticker.C:
			m.nodeCommandMap.Range(func(key, value interface{}) bool {
				messages := value.([]*controllerapi.ControlMessage)
				for i, msg := range messages {
					// Check if the message type matches
					fmt.Printf("payload type: %s\n", reflect.TypeOf(msg.GetPayload()))
					if reflect.TypeOf(msg.GetPayload()) == messageType {
						foundMessage = msg

						// remove the message from the command list
						messages = append(messages[:i], messages[i+1:]...)
						m.nodeCommandMap.Store(key, messages)

						return false // Stop the range loop
					}
				}
				return true // Continue the range loop
			})
			if foundMessage != nil {
				return foundMessage, nil // Exit early if we found a message
			}
		}
	}
}

// GetConnectionStatus implements Messaging.
func (m *messagingService) GetConnectionStatus(nodeID string) (NodeStatus, error) {
	status, ok := m.nodeConnectionStatusMap.Load(nodeID)
	if !ok {
		return NodeStatusUnknown, fmt.Errorf("no connection status found for node %s", nodeID)
	}
	return status.(NodeStatus), nil
}

// UpdateConnectionStatus implements Messaging.
func (m *messagingService) UpdateConnectionStatus(nodeID string, status NodeStatus) {
	m.nodeConnectionStatusMap.Store(nodeID, status)
}

// AddStream implements Messaging.
func (m *messagingService) AddStream(nodeID string, stream controllerapi.ControllerService_OpenControlChannelServer) {
	m.nodeStreamMap.Store(nodeID, stream)

	// Update status to connected
	m.UpdateConnectionStatus(nodeID, NodeStatusConnected)
}

// RemoveStream implements Messaging.
func (m *messagingService) RemoveStream(nodeID string) error {
	if _, ok := m.nodeStreamMap.LoadAndDelete(nodeID); !ok {
		return fmt.Errorf("no stream found for node %s", nodeID)
	}

	// Update status to not connected
	m.UpdateConnectionStatus(nodeID, NodeStatusNotConnected)

	return nil
}

// SendMessage implements Messaging.
func (m *messagingService) SendMessage(nodeID string, controlMessage *controllerapi.ControlMessage) error {
	stream, ok := m.nodeStreamMap.Load(nodeID)
	if !ok {
		return fmt.Errorf("no stream found for node %s", nodeID)
	}

	// Type assert the stream to the correct type
	controlChannelStream, ok := stream.(controllerapi.ControllerService_OpenControlChannelServer)
	if !ok {
		return fmt.Errorf("stream for node %s is not of type OpenControlChannelServer", nodeID)
	}

	// Send the message through the stream
	if err := controlChannelStream.Send(controlMessage); err != nil {
		return fmt.Errorf("failed to send message to node %s: %w", nodeID, err)
	}

	return nil
}

func NewMessagingService() Messaging {
	return &messagingService{}
}
