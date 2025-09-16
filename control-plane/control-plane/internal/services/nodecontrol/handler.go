package nodecontrol

import (
	"context"
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
	SendMessage(ctx context.Context, nodeID string, configurationCommand *controllerapi.ControlMessage) error
	AddStream(ctx context.Context, nodeID string, stream controllerapi.ControllerService_OpenControlChannelServer)
	RemoveStream(ctx context.Context, nodeID string) error
	GetConnectionStatus(ctx context.Context, nodeID string) (NodeStatus, error)
	UpdateConnectionStatus(nctx context.Context, odeID string, status NodeStatus)

	WaitForResponse(ctx context.Context, nodeID string,
		messageType reflect.Type, messageID string) (*controllerapi.ControlMessage, error)
	ResponseReceived(ctx context.Context, nodeID string, command *controllerapi.ControlMessage)
}
