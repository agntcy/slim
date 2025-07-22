package sbapiservice

import (
	"log"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	"github.com/google/uuid"
)

type SouthboundAPIServer interface {
	controllerapi.ControllerServiceServer
}

type sbAPIService struct {
	controllerapi.UnimplementedControllerServiceServer
	dbService        db.DataAccess
	messagingService nodecontrol.NodeCommandHandler
}

func NewSBAPIService(dbService db.DataAccess, messagingService nodecontrol.NodeCommandHandler) SouthboundAPIServer {
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

	// TODO: receive with timeout, if no register request received within a certain time, close the stream
	msg, err := stream.Recv()
	if err != nil {
		log.Printf("Error receiving message: %v", err)
		return err
	}

	registeredNodeID := ""

	// Check for ControlMessage_RegisterNodeRequest
	if regReq, ok := msg.Payload.(*controllerapi.ControlMessage_RegisterNodeRequest); ok {
		registeredNodeID = regReq.RegisterNodeRequest.NodeId
		log.Printf("Register node with ID: %v", registeredNodeID)
		s.dbService.SaveNode(db.Node{
			ID: registeredNodeID,
		})
		s.messagingService.AddStream(registeredNodeID, stream)
		s.messagingService.UpdateConnectionStatus(registeredNodeID, nodecontrol.NodeStatusConnected)
	}

	if registeredNodeID == "" {
		log.Println("No register node request received, closing stream.")
		return nil
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
				s.messagingService.UpdateConnectionStatus(nodeID, nodecontrol.NodeStatusNotConnected)

				err := s.messagingService.RemoveStream(nodeID)
				if err != nil {
					log.Printf("Error removing stream for node %s: %v", nodeID, err)
				}

				if err = s.messagingService.SendMessage(nodeID, &controllerapi.ControlMessage{
					MessageId: uuid.NewString(),
					Payload: &controllerapi.ControlMessage_DeregisterNodeResponse{
						DeregisterNodeResponse: &controllerapi.DeregisterNodeResponse{
							Success: true,
						},
					},
				}); err != nil {
					log.Printf("Error sending DeregisterNodeResponse to node %s: %v", nodeID, err)
				}
				return nil
			}
			continue
		case *controllerapi.ControlMessage_Ack:
			log.Printf("Received ACK for message ID: %s, Success: %t", msg.MessageId, payload.Ack.Success)
			s.messagingService.ResponseReceived(registeredNodeID, msg)
			continue
		case *controllerapi.ControlMessage_ConnectionListResponse:
			log.Printf("Received ConnectionListResponse for node %s: %v", registeredNodeID, payload.ConnectionListResponse)
			s.messagingService.ResponseReceived(registeredNodeID, msg)
		case *controllerapi.ControlMessage_SubscriptionListResponse:
			log.Printf("Received SubscriptionListResponse for node %s: %v", registeredNodeID, payload.SubscriptionListResponse)
			s.messagingService.ResponseReceived(registeredNodeID, msg)
		default:
			log.Printf("Invalid payload received from node %s: %s : %v", registeredNodeID, msg.MessageId, msg.Payload)
		}
	}
}
