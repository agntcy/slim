package messagingservice

import (
	"fmt"
	"sync"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
)

type Messaging interface {
	SendMessage(nodeID string, configurationCommand *controllerapi.ControlMessage) error
	ReceiveMessage(nodeID string) (*controllerapi.ControlMessage, error)
}

type messagingService struct {
	nodeCommandMap sync.Map // Maps node IDs and commands map[nodeID][]*controllerapi.ConfigurationCommand
}

// Check if a message exists for a node and the returns the first command in the queue.
func (m *messagingService) ReceiveMessage(nodeID string) (*controllerapi.ControlMessage, error) {
	value, ok := m.nodeCommandMap.Load(nodeID)
	if !ok {
		return nil, nil
	}
	commands, ok := value.([]*controllerapi.ControlMessage)
	if !ok {
		return nil, fmt.Errorf("no commands available for node %s", nodeID)
	}
	// Return the first command and remove it from the map
	command := commands[0]
	commands = commands[1:]
	if len(commands) == 0 {
		m.nodeCommandMap.Delete(nodeID)
	} else {
		m.nodeCommandMap.Store(nodeID, commands)
	}

	return command, nil
}

// Add a command to the node's command queue.
func (m *messagingService) SendMessage(nodeID string, configurationCommand *controllerapi.ControlMessage) error {
	// TODO check if node with nodeID is connected status
	// TODO wait for ACK from the node before returning
	value, ok := m.nodeCommandMap.Load(nodeID)
	var commands []*controllerapi.ControlMessage
	if ok {
		commands = value.([]*controllerapi.ControlMessage)
	}
	commands = append(commands, configurationCommand)
	m.nodeCommandMap.Store(nodeID, commands)

	return nil
}

func NewMessagingService() Messaging {
	return &messagingService{
		nodeCommandMap: sync.Map{},
	}
}
