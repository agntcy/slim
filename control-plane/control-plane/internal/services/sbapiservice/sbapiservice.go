package sbapiservice

import (
	"log"
	"sync"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
)

type SouthboundAPIServer interface {
	controllerapi.ControllerServiceServer
}

type sbAPIService struct {
	controllerapi.UnimplementedControllerServiceServer
	dbService         db.DataAccess
	nodeConnectionMap sync.Map
	nodeConnectionStatusMap sync.Map
}

// Node Status enumeration
type NodeStatus string

const (
	NodeStatusConnected   NodeStatus = "connected"
	NodeStatusNotConnected NodeStatus = "not_connected"
	NodeStatusUnknown  NodeStatus = "unknown"
)

func NewSBAPIService(dbService db.DataAccess) SouthboundAPIServer {
	return &sbAPIService{
		dbService: dbService,
	}
}

func (s *sbAPIService) GetNodeConnection(nodeID string) (controllerapi.ControllerService_OpenControlChannelServer, bool) {
	if conn, ok := s.nodeConnectionMap.Load(nodeID); ok {
		return conn.(controllerapi.ControllerService_OpenControlChannelServer), true
	}
	return nil, false
}

func (s *sbAPIService) GetNodeConnectionStatus(nodeID string) NodeStatus {
	if status, ok := s.nodeConnectionStatusMap.Load(nodeID); ok {
		return status.(NodeStatus)
	}
	return NodeStatusUnknown
}

func (s *sbAPIService) OpenControlChannel(stream controllerapi.ControllerService_OpenControlChannelServer) error {

	// Handle the control channel opening logic here
	// This is a placeholder implementation
	for {
		msg, err := stream.Recv()
		if err != nil {
			return err
		}
		// Process the received message
		if msg != nil {
			switch payload := msg.Payload.(type) {
			case *controllerapi.ControlMessage_DeregisterNodeRequest:
				// Handle deregistration logic
				if payload.DeregisterNodeRequest.Node != nil {
					nodeID := payload.DeregisterNodeRequest.Node.Id
					log.Printf("Deregister node with ID: %v", nodeID)
					// Update the node status to not connected
 					s.nodeConnectionStatusMap.Store(nodeID, NodeStatusNotConnected)
					s.nodeConnectionMap.Delete(nodeID)
				}

			case *controllerapi.ControlMessage_RegisterNodeRequest:
				// Handle registration logic
				if payload.RegisterNodeRequest != nil {
					nodeID := payload.RegisterNodeRequest.NodeId
					log.Printf("Register node with ID: %v", nodeID)
					s.dbService.SaveNode(db.Node{
						ID: nodeID,
					})
					s.nodeConnectionStatusMap.Store(nodeID, NodeStatusConnected)
					s.nodeConnectionMap.Store(nodeID, stream)
				}
			}

			if err := stream.Send(msg); err != nil {
				return err
			}
		}
	}
}
