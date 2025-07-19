package sbapiservice

import (
	"fmt"
	"log"
	"sync"
	"time"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/messagingservice"
)

type SouthboundAPIServer interface {
	controllerapi.ControllerServiceServer
}

type sbAPIService struct {
	controllerapi.UnimplementedControllerServiceServer
	dbService               db.DataAccess
	messagingService        messagingservice.Messaging
	nodeConnectionMap       sync.Map // Maps node ID to the open control channel stream
	nodeConnectionStatusMap sync.Map // Maps node ID to its connection status
}

// Node Status enumeration
type NodeStatus string

const (
	NodeStatusConnected    NodeStatus = "connected"
	NodeStatusNotConnected NodeStatus = "not_connected"
	NodeStatusUnknown      NodeStatus = "unknown"
)

func NewSBAPIService(dbService db.DataAccess, messagingService messagingservice.Messaging) SouthboundAPIServer {
	return &sbAPIService{
		dbService:        dbService,
		messagingService: messagingService,
	}
}

func (s *sbAPIService) OpenControlChannel(stream controllerapi.ControllerService_OpenControlChannelServer) error {

	type recvResult struct {
		msg *controllerapi.ControlMessage
		err error
	}

	// For testing
	// TODO: remove this after testing
	s.nodeConnectionMap.Store(stream, "node1")

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
						s.nodeConnectionMap.Delete(stream)
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
						s.nodeConnectionMap.Store(stream, nodeID)
					}
				default:
					log.Printf("Unknown message: %v", res.msg)
				}
			}
		default:

			// Check if there is new message for the node
			streamNodeID, ok := s.nodeConnectionMap.Load(stream)
			if !ok {
				log.Printf("No node ID found for the stream")
				continue
			}
			nodeID := streamNodeID.(string)
			command, err := s.messagingService.ReceiveMessage(nodeID)
			if err != nil {
				log.Printf("Failed to receive message for node %s: %v", nodeID, err)
				continue
			}
			if command != nil {
				log.Printf("Sending command to node %s: %v", nodeID, command)
				stream.Send(command)
			} else {
				log.Printf("No command available for node %s", nodeID)
			}

			time.Sleep(10 * time.Second) // Avoid busy loop
		}
	}
}
