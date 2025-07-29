package nodecontrol

import (
	"fmt"
	"reflect"
	"sync"
	"time"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
)

type defaultNodeCommandHandler struct {
	// Maps node IDs and streams map[nodeID]controllerapi.ControllerService_OpenControlChannelServer
	nodeStreamMap sync.Map
	// Maps node ID to its connection status
	nodeConnectionStatusMap sync.Map
	// Contains send locks for each node ID to serialize SendMessage calls
	nodeSendLockMap sync.Map // Maps node IDs to their send locks
	// Maps contains received *controllerapi.ControlMessage responses for a node IDs and message type
	nodeResponseMsgMap sync.Map
}

// WaitForResponse implements NodeCommandHandler.
func (m *defaultNodeCommandHandler) WaitForResponse(
	nodeID string, messageType reflect.Type,
) (*controllerapi.ControlMessage, error) {
	if nodeID == "" {
		return nil, fmt.Errorf("nodeID cannot be empty")
	}
	if messageType == nil {
		return nil, fmt.Errorf("messageType cannot be nil")
	}

	// create a channel to receive *controllerapi.ControlMessage messages.
	// save the channel in m.nodeResponseMsgMap with key = nodeID + messageType.
	key := nodeID + ":" + messageType.String()
	ch := make(chan *controllerapi.ControlMessage, 1)
	m.nodeResponseMsgMap.Store(key, ch)
	defer m.nodeResponseMsgMap.Delete(key)

	fmt.Println("Waiting for message of type:", messageType)

	// wait on that channel with timeout
	select {
	case msg := <-ch:
		return msg, nil
	case <-time.After(10 * time.Second):
		return nil, fmt.Errorf("timeout waiting for message of type %v", messageType)
	}
}

// ResponseReceived implements NodeCommandHandler.
func (m *defaultNodeCommandHandler) ResponseReceived(nodeID string, command *controllerapi.ControlMessage) {
	if nodeID == "" {
		return
	}
	if command == nil {
		return
	}
	if command.MessageId == "" {
		return
	}

	// Get the channel for the specific nodeID and message type
	key := nodeID + ":" + reflect.TypeOf(command.GetPayload()).String()
	ch, ok := m.nodeResponseMsgMap.Load(key)
	if !ok {
		fmt.Printf("No channel found for node %s and message type %s\n", nodeID, reflect.TypeOf(command.GetPayload()))
		return
	}

	// Send the command to the channel
	select {
	case ch.(chan *controllerapi.ControlMessage) <- command:
	default:
		fmt.Printf(
			"Channel for node %s and message type %s is full, dropping message\n",
			nodeID,
			reflect.TypeOf(command.GetPayload()),
		)
	}
}

// GetConnectionStatus implements NodeCommandHandler.
func (m *defaultNodeCommandHandler) GetConnectionStatus(nodeID string) (NodeStatus, error) {
	status, ok := m.nodeConnectionStatusMap.Load(nodeID)
	if !ok {
		return NodeStatusUnknown, fmt.Errorf("no connection status found for node %s", nodeID)
	}
	return status.(NodeStatus), nil
}

// UpdateConnectionStatus implements NodeCommandHandler.
func (m *defaultNodeCommandHandler) UpdateConnectionStatus(nodeID string, status NodeStatus) {
	m.nodeConnectionStatusMap.Store(nodeID, status)
}

// AddStream implements NodeCommandHandler.
func (m *defaultNodeCommandHandler) AddStream(
	nodeID string, stream controllerapi.ControllerService_OpenControlChannelServer,
) {
	m.nodeStreamMap.Store(nodeID, stream)

	// Update status to connected
	m.UpdateConnectionStatus(nodeID, NodeStatusConnected)
}

// RemoveStream implements NodeCommandHandler.
func (m *defaultNodeCommandHandler) RemoveStream(nodeID string) error {
	if _, ok := m.nodeStreamMap.LoadAndDelete(nodeID); !ok {
		return fmt.Errorf("no stream found for node %s", nodeID)
	}

	// Update status to not connected
	m.UpdateConnectionStatus(nodeID, NodeStatusNotConnected)

	return nil
}

// SendMessage implements NodeCommandHandler.
func (m *defaultNodeCommandHandler) SendMessage(nodeID string, controlMessage *controllerapi.ControlMessage) error {
	// check if nodeID is empty
	if nodeID == "" {
		return fmt.Errorf("nodeID cannot be empty")
	}

	// Use a per-nodeID mutex to serialize SendMessage calls for the same nodeID
	mutexIface, _ := m.nodeSendLockMap.LoadOrStore(nodeID+"_send_lock", &sync.Mutex{})
	nodeMutex := mutexIface.(*sync.Mutex)

	nodeMutex.Lock()
	defer nodeMutex.Unlock()

	// check status of the node
	status, err := m.GetConnectionStatus(nodeID)
	if err != nil {
		return fmt.Errorf("failed to get connection status for node %s: %w", nodeID, err)
	}
	if status != NodeStatusConnected {
		return fmt.Errorf("node %s is not connected, current status: %s", nodeID, status)
	}

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

func DefaultNodeCommandHandler() NodeCommandHandler {
	return &defaultNodeCommandHandler{}
}
