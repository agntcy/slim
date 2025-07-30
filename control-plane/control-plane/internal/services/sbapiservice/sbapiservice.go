package sbapiservice

import (
	"context"
	"time"

	"github.com/google/uuid"
	"github.com/rs/zerolog"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	"github.com/agntcy/slim/control-plane/control-plane/internal/util"
)

type SouthboundAPIServer interface {
	controllerapi.ControllerServiceServer
}

type sbAPIService struct {
	config config.APIConfig
	controllerapi.UnimplementedControllerServiceServer
	dbService                 db.DataAccess
	messagingService          nodecontrol.NodeCommandHandler
	nodeRegistrationListeners []nodecontrol.NodeRegistrationHandler
}

func NewSBAPIService(
	config config.APIConfig,
	dbService db.DataAccess,
	cmdHandler nodecontrol.NodeCommandHandler,
	nodeRegistrationListeners []nodecontrol.NodeRegistrationHandler,
) SouthboundAPIServer {
	return &sbAPIService{
		config:                    config,
		dbService:                 dbService,
		messagingService:          cmdHandler,
		nodeRegistrationListeners: nodeRegistrationListeners,
	}
}

func (s *sbAPIService) OpenControlChannel(stream controllerapi.ControllerService_OpenControlChannelServer) error {
	ctx := util.GetContextWithLogger(context.Background(), s.config.LogConfig)
	zlog := zerolog.Ctx(ctx)

	// Acknowledge the connection
	messageID := uuid.NewString()
	msg := &controllerapi.ControlMessage{
		MessageId: messageID,
		Payload: &controllerapi.ControlMessage_Ack{
			Ack: &controllerapi.Ack{
				Success: true,
			},
		},
	}
	err := stream.Send(msg)
	if err != nil {
		zlog.Error().Msgf("Error sending message: %v", err)
		return err
	}

	// if no register request received within a certain time, close the stream
	recvTimeout := 15 * time.Second // assume this is a time.Duration field in your config
	recvCtx, cancel := context.WithTimeout(ctx, recvTimeout)
	defer cancel()

	msgCh := make(chan *controllerapi.ControlMessage, 1)
	errCh := make(chan error, 1)

	go func() {
		rmsg, rerr := stream.Recv()
		if err != nil {
			errCh <- rerr
			return
		}
		msgCh <- rmsg
	}()

	select {
	case <-recvCtx.Done():
		zlog.Error().Msgf("Timeout waiting for register node request")
		return recvCtx.Err()
	case rerr := <-errCh:
		zlog.Error().Msgf("Error receiving message: %v", err)
		return rerr
	case m := <-msgCh:
		msg = m
	}

	registeredNodeID := ""

	// Check for ControlMessage_RegisterNodeRequest
	if regReq, ok := msg.Payload.(*controllerapi.ControlMessage_RegisterNodeRequest); ok {
		registeredNodeID = regReq.RegisterNodeRequest.NodeId
		zlog.Info().Msgf("Registering node with ID: %v", registeredNodeID)
		_, err = s.dbService.SaveNode(db.Node{
			ID: registeredNodeID,
		})
		if err != nil {
			zlog.Error().Msgf("Error saving node: %v", err)
			return err
		}
		s.messagingService.AddStream(registeredNodeID, stream)
		s.messagingService.UpdateConnectionStatus(registeredNodeID, nodecontrol.NodeStatusConnected)

		// Acknowledge the registration
		ackMsg := &controllerapi.ControlMessage{
			MessageId: uuid.NewString(),
			Payload: &controllerapi.ControlMessage_Ack{
				Ack: &controllerapi.Ack{
					Success:  true,
					Messages: []string{"Node registered successfully"},
				},
			},
		}
		_ = stream.Send(ackMsg)

		// call all registered listeners in a goroutine
		go func() {
			for _, listener := range s.nodeRegistrationListeners {
				if err := listener.NodeRegistered(ctx, registeredNodeID); err != nil {
					zlog.Error().Msgf("Error in node registration listener: %v", err)
				}
			}
		}()
	}

	if registeredNodeID == "" {
		zlog.Info().Msgf("No register node request received, closing stream.")
		return nil
	}

	return s.handleNodeMessages(stream, zlog, registeredNodeID)
}

func (s *sbAPIService) handleNodeMessages(stream controllerapi.ControllerService_OpenControlChannelServer,
	zlog *zerolog.Logger, registeredNodeID string) error {
	for {
		msg, err := stream.Recv()
		if err != nil {
			zlog.Error().Msgf("Stream connection failed for node %s: %v", registeredNodeID, err)

			// Update the node status to not connected
			s.messagingService.UpdateConnectionStatus(registeredNodeID, nodecontrol.NodeStatusNotConnected)
			zlog.Error().Msgf("Node %s status set to: %v", registeredNodeID, nodecontrol.NodeStatusNotConnected)

			err := s.messagingService.RemoveStream(registeredNodeID)
			if err != nil {
				zlog.Error().Msgf("Error removing stream for node %s: %v", registeredNodeID, err)
			}

			return err
		}
		switch payload := msg.Payload.(type) {
		case *controllerapi.ControlMessage_DeregisterNodeRequest:
			// Handle deregistration logic
			if payload.DeregisterNodeRequest.Node != nil {
				nodeID := payload.DeregisterNodeRequest.Node.Id
				zlog.Info().Msgf("Deregister node with ID: %v", nodeID)
				// Update the node status to not connected
				s.messagingService.UpdateConnectionStatus(nodeID, nodecontrol.NodeStatusNotConnected)

				err := s.messagingService.RemoveStream(nodeID)
				if err != nil {
					zlog.Error().Msgf("Error removing stream for node %s: %v", nodeID, err)
				}

				if err = s.messagingService.SendMessage(nodeID, &controllerapi.ControlMessage{
					MessageId: uuid.NewString(),
					Payload: &controllerapi.ControlMessage_DeregisterNodeResponse{
						DeregisterNodeResponse: &controllerapi.DeregisterNodeResponse{
							Success: true,
						},
					},
				}); err != nil {
					zlog.Error().Msgf("Error sending DeregisterNodeResponse to node %s: %v", nodeID, err)
				}
				return nil
			}
			continue
		case *controllerapi.ControlMessage_Ack:
			zlog.Debug().Msgf("Received ACK for message ID: %s, Success: %t", msg.MessageId, payload.Ack.Success)
			s.messagingService.ResponseReceived(registeredNodeID, msg)
			continue
		case *controllerapi.ControlMessage_ConnectionListResponse:
			zlog.Debug().Msgf(
				"Received ConnectionListResponse for node %s: %v",
				registeredNodeID, payload.ConnectionListResponse,
			)
			s.messagingService.ResponseReceived(registeredNodeID, msg)
		case *controllerapi.ControlMessage_SubscriptionListResponse:
			zlog.Debug().Msgf(
				"Received SubscriptionListResponse for node %s: %v",
				registeredNodeID, payload.SubscriptionListResponse)
			s.messagingService.ResponseReceived(registeredNodeID, msg)
		default:
			zlog.Debug().Msgf(
				"Invalid payload received from node %s: %s : %v",
				registeredNodeID, msg.MessageId, msg.Payload,
			)
		}
	}
}
