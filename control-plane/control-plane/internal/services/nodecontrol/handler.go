package nodecontrol

import (
	"reflect"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
)

const (
	NodeStatusConnected    NodeStatus = "connected"
	NodeStatusNotConnected NodeStatus = "not_connected"
	NodeStatusUnknown      NodeStatus = "unknown"
)

// Node Status enumeration
type NodeStatus string

type NodeCommandHandler interface {
	SendMessage(nodeID string, configurationCommand *controllerapi.ControlMessage) error
	AddStream(nodeID string, stream controllerapi.ControllerService_OpenControlChannelServer)
	RemoveStream(nodeID string) error
	GetConnectionStatus(nodeID string) (NodeStatus, error)
	UpdateConnectionStatus(nodeID string, status NodeStatus)

	WaitForResponse(nodeID string, messageType reflect.Type) (*controllerapi.ControlMessage, error)
	ResponseReceived(nodeID string, command *controllerapi.ControlMessage)
}
