package sbapiservice

import (
	"fmt"
	"log"
	"sync"
	"time"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
)

type SouthboundAPIServer interface {
	controllerapi.ControllerServiceServer
}

type sbAPIService struct {
	controllerapi.UnimplementedControllerServiceServer
	dbService               db.DataAccess
	nodeConnectionMap       sync.Map
	nodeConnectionStatusMap sync.Map
}

// Node Status enumeration
type NodeStatus string

const (
	NodeStatusConnected    NodeStatus = "connected"
	NodeStatusNotConnected NodeStatus = "not_connected"
	NodeStatusUnknown      NodeStatus = "unknown"
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

	type recvResult struct {
		msg *controllerapi.ControlMessage
		err error
	}

	recvChan := make(chan recvResult)

	go func() {
		for {
			msg, err := stream.Recv()
			recvChan <- recvResult{msg, err}
			if err != nil {
				close(recvChan)
				return
			}
		}
	}()

	for {
		select {
		case res, _ := <-recvChan:
			if res.err != nil {
				fmt.Printf("Recv error: %v", res.err)
				return res.err
			}
			if res.msg != nil {
				switch payload := res.msg.Payload.(type) {
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
				default:
					log.Printf("Unknown message: %v", res.msg)
				}
			}
		default:
			fmt.Print("cica")
			time.Sleep(10 * time.Millisecond) // Avoid busy loop
		}
	}
}
