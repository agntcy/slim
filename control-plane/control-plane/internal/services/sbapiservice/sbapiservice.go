package sbapiservice

import (
	"log"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
)

type SouthboundAPIServer interface {
	controllerapi.ControllerServiceServer
}

type sbAPIService struct {
	controllerapi.UnimplementedControllerServiceServer
	dbService db.DataAccess
}

func NewSBAPIService(dbService db.DataAccess) SouthboundAPIServer {
	return &sbAPIService{
		dbService: dbService,
	}
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
		// For now, just log it
		if msg != nil {
			switch payload := msg.Payload.(type) {
			case *controllerapi.ControlMessage_DeregisterNodeRequest:
				// Handle deregistration logic
				if payload.DeregisterNodeRequest.Node != nil {
					nodeID := payload.DeregisterNodeRequest.Node.Id
					if err := s.dbService.DeleteNode(nodeID); err != nil {
						return err
					}
				}

			case *controllerapi.ControlMessage_RegisterNodeRequest:
				// Handle registration logic
				log.Printf("Received RegisterNodeRequest: %v", payload)
			}

			if err := stream.Send(msg); err != nil {
				return err
			}
		}
	}
}
