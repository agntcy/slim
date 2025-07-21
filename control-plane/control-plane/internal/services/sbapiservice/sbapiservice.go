package sbapiservice

import (
	"log"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/messagingservice"
	"github.com/google/uuid"
)

type SouthboundAPIServer interface {
	controllerapi.ControllerServiceServer
}

type sbAPIService struct {
	controllerapi.UnimplementedControllerServiceServer
	dbService        db.DataAccess
	messagingService messagingservice.Messaging
}

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

	// Acknowledge the connection
	messageId := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageId,
		Payload: &controllerapi.ControlMessage_Ack{
			Ack: &controllerapi.Ack{
				Success: true,
			},
		},
	}
	stream.Send(msg)

	// For testing
	// TODO: remove this after testing
	//s.nodeConnectionMap.Store(stream, "node1")

	msg, err := stream.Recv()
	if err != nil {
		log.Printf("Error receiving message: %v", err)
		return err
	}

	// Check for ControlMessage_RegisterNodeRequest
	if regReq, ok := msg.Payload.(*controllerapi.ControlMessage_RegisterNodeRequest); ok {
		nodeID := regReq.RegisterNodeRequest.NodeId
		log.Printf("Register node with ID: %v", nodeID)
		s.dbService.SaveNode(db.Node{
			ID: nodeID,
		})
		s.messagingService.AddStream(nodeID, stream)
		s.messagingService.UpdateConnectionStatus(nodeID, messagingservice.NodeStatusConnected)
	}

	for {
		msg, err := stream.Recv()
		if err != nil {
			log.Printf("Error receiving message: %v", err)
			return err
		}
		switch payload := msg.Payload.(type) {
		case *controllerapi.ControlMessage_DeregisterNodeRequest:
			// Handle deregistration logic
			if payload.DeregisterNodeRequest.Node != nil {
				nodeID := payload.DeregisterNodeRequest.Node.Id
				log.Printf("Deregister node with ID: %v", nodeID)
				// Update the node status to not connected
				s.messagingService.UpdateConnectionStatus(nodeID, messagingservice.NodeStatusNotConnected)
				s.messagingService.RemoveStream(nodeID)
			}
			/*
				case *controllerapi.ControlMessage_Ack:
					log.Printf("Received ACK for message ID: %s, Success: %t", msg.MessageId, payload.Ack.Success) */

		default:
			nodeID, err := s.messagingService.GetNodeId(stream)
			if err != nil {
				log.Printf("Error getting node ID: %v", err)
				return err
			}

			s.messagingService.AddNodeCommand(nodeID, msg)

			log.Printf("Received message from node %s: %s", nodeID, msg.MessageId)
		}
	}
}
