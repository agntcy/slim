package sbapiservice

import (
	"context"
	"net"
	"strconv"
	"time"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/groupservice"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	"github.com/agntcy/slim/control-plane/control-plane/internal/util"
	"github.com/google/uuid"
	"github.com/rs/zerolog"
	"google.golang.org/grpc/peer"
)

type SouthboundAPIServer interface {
	controllerapi.ControllerServiceServer
}

type sbAPIService struct {
	config config.APIConfig
	controllerapi.UnimplementedControllerServiceServer
	dbService                 db.DataAccess
	nodeCommandHandler        nodecontrol.NodeCommandHandler
	nodeRegistrationListeners []nodecontrol.NodeRegistrationHandler
	groupservice              *groupservice.GroupService
}

func NewSBAPIService(
	config config.APIConfig,
	dbService db.DataAccess,
	cmdHandler nodecontrol.NodeCommandHandler,
	nodeRegistrationListeners []nodecontrol.NodeRegistrationHandler,
	groupservice *groupservice.GroupService,
) SouthboundAPIServer {
	return &sbAPIService{
		config:                    config,
		dbService:                 dbService,
		nodeCommandHandler:        cmdHandler,
		nodeRegistrationListeners: nodeRegistrationListeners,
		groupservice:              groupservice,
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
		if rerr != nil {
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

		// Extract host and port from gRPC peer info
		var host string
		var port uint32
		if peerInfo, ok := peer.FromContext(stream.Context()); ok {
			switch addr := peerInfo.Addr.(type) {
			case *net.TCPAddr:
				host = addr.IP.String()
				if addr.Port >= 0 && addr.Port <= 65535 {
					port = uint32(addr.Port)
				}
			default:
				hostStr, portStr, splitErr := net.SplitHostPort(peerInfo.Addr.String())
				if splitErr == nil {
					host = hostStr
					if p, parseErr := strconv.ParseUint(portStr, 10, 16); parseErr == nil { // validated to 0-65535 by bit size 16
						port = uint32(p)
					}
				}
			}
		}

		_, err = s.dbService.SaveNode(db.Node{
			ID:   registeredNodeID,
			Host: host,
			Port: port,
		})
		if err != nil {
			zlog.Error().Msgf("Error saving node: %v", err)
			return err
		}
		s.nodeCommandHandler.AddStream(registeredNodeID, stream)
		s.nodeCommandHandler.UpdateConnectionStatus(registeredNodeID, nodecontrol.NodeStatusConnected)

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
			s.nodeCommandHandler.UpdateConnectionStatus(registeredNodeID, nodecontrol.NodeStatusNotConnected)
			zlog.Error().Msgf("Node %s status set to: %v", registeredNodeID, nodecontrol.NodeStatusNotConnected)

			err := s.nodeCommandHandler.RemoveStream(registeredNodeID)
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
				s.nodeCommandHandler.UpdateConnectionStatus(nodeID, nodecontrol.NodeStatusNotConnected)

				err := s.nodeCommandHandler.RemoveStream(nodeID)
				if err != nil {
					zlog.Error().Msgf("Error removing stream for node %s: %v", nodeID, err)
				}

				if err = s.nodeCommandHandler.SendMessage(nodeID, &controllerapi.ControlMessage{
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
			s.nodeCommandHandler.ResponseReceived(registeredNodeID, msg)
			continue
		case *controllerapi.ControlMessage_ConnectionListResponse:
			zlog.Debug().Msgf(
				"Received ConnectionListResponse for node %s: %v",
				registeredNodeID, payload.ConnectionListResponse,
			)
			s.nodeCommandHandler.ResponseReceived(registeredNodeID, msg)
		case *controllerapi.ControlMessage_SubscriptionListResponse:
			zlog.Debug().Msgf(
				"Received SubscriptionListResponse for node %s: %v",
				registeredNodeID, payload.SubscriptionListResponse)
			s.nodeCommandHandler.ResponseReceived(registeredNodeID, msg)

		case *controllerapi.ControlMessage_CreateChannelRequest:
			zlog.Debug().Msgf(
				"Received CreateChannelRequest for node %s: %v", registeredNodeID, payload.CreateChannelRequest,
			)
			resp, err := s.groupservice.CreateChannel(
				util.GetContextWithLogger(context.Background(), s.config.LogConfig),
				payload.CreateChannelRequest,
				&controlplaneApi.NodeEntry{},
			)
			if err != nil {
				zlog.Error().Msgf("Error creating channel: %v", err)
				return err
			}
			ackMsg := &controllerapi.ControlMessage{
				MessageId: uuid.NewString(),
				Payload: &controllerapi.ControlMessage_CreateChannelResponse{
					CreateChannelResponse: resp,
				},
			}
			if err := s.nodeCommandHandler.SendMessage(registeredNodeID, ackMsg); err != nil {
				zlog.Error().Msgf("Error sending CreateChannelResponse to node %s: %v", registeredNodeID, err)
				return err
			}
			zlog.Info().Msgf("Channel created successfully for node %s: %s", registeredNodeID, resp.ChannelId)

		case *controllerapi.ControlMessage_DeleteChannelRequest:
			zlog.Debug().Msgf(
				"Received DeleteChannelRequest for node %s: %v", registeredNodeID, payload.DeleteChannelRequest,
			)
			resp, err := s.groupservice.DeleteChannel(
				util.GetContextWithLogger(context.Background(), s.config.LogConfig),
				payload.DeleteChannelRequest,
			)
			if err != nil {
				zlog.Error().Msgf("Error deleting channel: %v", err)
				return err
			}
			ackMsg := &controllerapi.ControlMessage{
				MessageId: uuid.NewString(),
				Payload: &controllerapi.ControlMessage_Ack{
					Ack: &controllerapi.Ack{
						Success:           resp.Success,
						OriginalMessageId: msg.MessageId,
					},
				},
			}
			if err := s.nodeCommandHandler.SendMessage(registeredNodeID, ackMsg); err != nil {
				zlog.Error().Msgf("Error sending DeleteChannelResponse to node %s: %v", registeredNodeID, err)
				return err
			}
			zlog.Info().Msgf("Channel deleted successfully for node %s", registeredNodeID)

		case *controllerapi.ControlMessage_AddParticipantRequest:
			zlog.Debug().Msgf(
				"Received AddParticipantRequest for node %s: %v", registeredNodeID, payload.AddParticipantRequest,
			)
			resp, err := s.groupservice.AddParticipant(
				util.GetContextWithLogger(context.Background(), s.config.LogConfig),
				payload.AddParticipantRequest,
			)
			if err != nil {
				zlog.Error().Msgf("Error adding participant: %v", err)
				return err
			}
			ackMsg := &controllerapi.ControlMessage{
				MessageId: uuid.NewString(),
				Payload: &controllerapi.ControlMessage_Ack{
					Ack: &controllerapi.Ack{
						Success:           resp.Success,
						OriginalMessageId: msg.MessageId,
					},
				},
			}
			if err := s.nodeCommandHandler.SendMessage(registeredNodeID, ackMsg); err != nil {
				zlog.Error().Msgf("Error sending AddParticipantResponse to node %s: %v", registeredNodeID, err)
				return err
			}
			zlog.Info().Msgf(
				"Participant added successfully for node %s: %s", registeredNodeID, payload.AddParticipantRequest.ParticipantId)

		case *controllerapi.ControlMessage_DeleteParticipantRequest:
			zlog.Debug().Msgf(
				"Received DeleteParticipantRequest for node %s: %v", registeredNodeID, payload.DeleteParticipantRequest,
			)
			resp, err := s.groupservice.DeleteParticipant(
				util.GetContextWithLogger(context.Background(), s.config.LogConfig),
				payload.DeleteParticipantRequest,
			)
			if err != nil {
				zlog.Error().Msgf("Error deleting participant: %v", err)
				return err
			}
			ackMsg := &controllerapi.ControlMessage{
				MessageId: uuid.NewString(),
				Payload: &controllerapi.ControlMessage_Ack{
					Ack: &controllerapi.Ack{
						Success:           resp.Success,
						OriginalMessageId: msg.MessageId,
					},
				},
			}
			if err := s.nodeCommandHandler.SendMessage(registeredNodeID, ackMsg); err != nil {
				zlog.Error().Msgf("Error sending DeleteParticipantResponse to node %s: %v", registeredNodeID, err)
				return err
			}
			zlog.Info().Msgf(
				"Participant deleted successfully for node %s: %s",
				registeredNodeID,
				payload.DeleteParticipantRequest.ParticipantId,
			)

		case *controllerapi.ControlMessage_ListChannelRequest:
			zlog.Debug().Msgf(
				"Received ListChannelRequest for node %s: %v", registeredNodeID, payload.ListChannelRequest,
			)
			resp, err := s.groupservice.ListChannels(
				util.GetContextWithLogger(context.Background(), s.config.LogConfig),
				payload.ListChannelRequest,
			)
			if err != nil {
				zlog.Error().Msgf("Error listing channels: %v", err)
				return err
			}
			ackMsg := &controllerapi.ControlMessage{
				MessageId: uuid.NewString(),
				Payload: &controllerapi.ControlMessage_ListChannelResponse{
					ListChannelResponse: resp,
				},
			}
			if err := s.nodeCommandHandler.SendMessage(registeredNodeID, ackMsg); err != nil {
				zlog.Error().Msgf("Error sending ListChannelResponse to node %s: %v", registeredNodeID, err)
				return err
			}
			zlog.Info().Msgf("Channels listed successfully for node %s", registeredNodeID)

		case *controllerapi.ControlMessage_ListParticipantsRequest:
			zlog.Debug().Msgf(
				"Received ListParticipantsRequest for node %s: %v", registeredNodeID, payload.ListParticipantsRequest,
			)
			resp, err := s.groupservice.ListParticipants(
				util.GetContextWithLogger(context.Background(), s.config.LogConfig),
				payload.ListParticipantsRequest,
			)
			if err != nil {
				zlog.Error().Msgf("Error listing participants: %v", err)
				return err
			}
			ackMsg := &controllerapi.ControlMessage{
				MessageId: uuid.NewString(),
				Payload: &controllerapi.ControlMessage_ListParticipantsResponse{
					ListParticipantsResponse: resp,
				},
			}
			if err := s.nodeCommandHandler.SendMessage(registeredNodeID, ackMsg); err != nil {
				zlog.Error().Msgf("Error sending ListParticipantsResponse to node %s: %v", registeredNodeID, err)
				return err
			}
			zlog.Info().Msgf("Participants listed successfully for node %s", registeredNodeID)

		default:
			zlog.Debug().Msgf(
				"Invalid payload received from node %s: %s : %v",
				registeredNodeID, msg.MessageId, msg.Payload,
			)
		}
	}
}
