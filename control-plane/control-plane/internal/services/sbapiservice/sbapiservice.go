package sbapiservice

import (
	"context"
	"fmt"
	"net"
	"time"

	"github.com/google/uuid"
	"github.com/rs/zerolog"
	"github.com/spiffe/go-spiffe/v2/spiffegrpc/grpccredentials"
	"google.golang.org/grpc/peer"

	controllerapi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/groupservice"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/routes"
	"github.com/agntcy/slim/control-plane/control-plane/internal/util"
)

type SouthboundAPIServer interface {
	controllerapi.ControllerServiceServer
}

type sbAPIService struct {
	config    config.APIConfig
	logConfig config.LogConfig
	controllerapi.UnimplementedControllerServiceServer
	dbService          db.DataAccess
	nodeCommandHandler nodecontrol.NodeCommandHandler
	routeService       *routes.RouteService
	groupservice       *groupservice.GroupService
}

func NewSBAPIService(config config.APIConfig,
	logConfig config.LogConfig,
	dbService db.DataAccess,
	cmdHandler nodecontrol.NodeCommandHandler,
	routeService *routes.RouteService,
	groupservice *groupservice.GroupService) SouthboundAPIServer {
	return &sbAPIService{
		config:             config,
		logConfig:          logConfig,
		dbService:          dbService,
		nodeCommandHandler: cmdHandler,
		routeService:       routeService,
		groupservice:       groupservice,
	}
}

func (s *sbAPIService) OpenControlChannel(stream controllerapi.ControllerService_OpenControlChannelServer) error {
	ctx := util.GetContextWithLogger(context.Background(), s.logConfig)
	zlog := zerolog.Ctx(ctx).With().Str("svc", "southbound").Logger()

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
		if regReq.RegisterNodeRequest.GroupName != nil && *regReq.RegisterNodeRequest.GroupName != "" {
			registeredNodeID = *regReq.RegisterNodeRequest.GroupName + "/" + registeredNodeID
		}
		zlog.Info().Msgf("Registering node with ID: %v", registeredNodeID)

		connDetails := make([]db.ConnectionDetails, 0, len(regReq.RegisterNodeRequest.ConnectionDetails))
		// Extract host from gRPC peer info
		host := getPeerHost(stream)
		for _, detail := range regReq.RegisterNodeRequest.ConnectionDetails {
			connDetails = append(connDetails, getConnDetails(host, detail))
			zlog.Info().Msgf("Connection details: %v", connDetails)
		}

		_, err = s.dbService.SaveNode(db.Node{
			ID:          registeredNodeID,
			GroupName:   regReq.RegisterNodeRequest.GroupName,
			ConnDetails: connDetails,
		})
		if err != nil {
			zlog.Error().Msgf("Error saving node: %v", err)
			return err
		}
		s.nodeCommandHandler.AddStream(ctx, registeredNodeID, stream)
		s.nodeCommandHandler.UpdateConnectionStatus(ctx, registeredNodeID, nodecontrol.NodeStatusConnected)

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

		s.routeService.NodeRegistered(ctx, registeredNodeID)
	}

	if registeredNodeID == "" {
		zlog.Info().Msgf("No register node request received, closing stream.")
		return nil
	}

	return s.handleNodeMessages(ctx, stream, registeredNodeID)
}

func getConnDetails(host string, detail *controllerapi.ConnectionDetails) db.ConnectionDetails {
	// use local endpoint if provided, otherwise use peer host
	endPoint := host
	if detail.LocalEndpoint != nil {
		endPoint = *detail.LocalEndpoint
	}
	// append port if provided in endpoint
	_, port, splitErr := net.SplitHostPort(detail.Endpoint)
	if splitErr == nil {
		endPoint = endPoint + ":" + port
	}

	connDetails := db.ConnectionDetails{
		Endpoint:         endPoint,
		MTLSRequired:     detail.MtlsRequired,
		GroupName:        detail.GroupName,
		ExternalEndpoint: detail.ExternalEndpoint,
	}
	return connDetails
}

func getPeerHost(stream controllerapi.ControllerService_OpenControlChannelServer) string {
	var host string
	if peerInfo, ok := peer.FromContext(stream.Context()); ok {
		switch addr := peerInfo.Addr.(type) {
		case *net.TCPAddr:
			host = addr.IP.String()
		default:
			hostStr, _, splitErr := net.SplitHostPort(peerInfo.Addr.String())
			if splitErr == nil {
				host = hostStr
			}
		}
		sid, ok := grpccredentials.PeerIDFromPeer(peerInfo)
		if ok {
			trustDomain := sid.TrustDomain().String()
			fmt.Println("---------------------------------- Trust Domain: ", trustDomain)
			fmt.Println("---------------------------------- SpiffeID: ", sid.String())
		} else {
			fmt.Println("no SPIFFE ID found")
		}
	}
	return host
}

func (s *sbAPIService) handleNodeMessages(ctx context.Context,
	stream controllerapi.ControllerService_OpenControlChannelServer,
	registeredNodeID string) error {
	zlog := zerolog.Ctx(ctx).With().Str("fn", "handleNodeMessages").Str("node_id", registeredNodeID).Logger()
	for {
		msg, err := stream.Recv()
		if err != nil {
			zlog.Error().Msgf("Stream connection failed: %v", err)

			// Update the node status to not connected
			s.nodeCommandHandler.UpdateConnectionStatus(ctx, registeredNodeID, nodecontrol.NodeStatusNotConnected)
			zlog.Error().Msgf("Node status set to: %v", nodecontrol.NodeStatusNotConnected)

			err := s.nodeCommandHandler.RemoveStream(ctx, registeredNodeID)
			if err != nil {
				zlog.Error().Msgf("Error removing stream for node: %v", err)
			}

			return err
		}
		switch payload := msg.Payload.(type) {
		case *controllerapi.ControlMessage_ConfigCommand:
			zlog.Debug().Msg("Received subscription update from node")
			// list subscriptions to add or remove in debug log
			for _, sub := range payload.ConfigCommand.SubscriptionsToSet {
				zlog.Debug().Msgf("Subscription to set: %v", sub)
				if sub.ConnectionId == "n/a" {
					zlog.Debug().Msgf("Create routes on all other nodes for subscription: %v", sub)
					_, err := s.routeService.AddRoute(ctx, routes.Route{
						SourceNodeID: routes.AllNodesID,
						DestNodeID:   registeredNodeID,
						Component0:   sub.Component_0,
						Component1:   sub.Component_1,
						Component2:   sub.Component_2,
						ComponentID:  sub.Id,
					})
					if err != nil {
						zlog.Error().Msgf("Error adding route: %v", err)
					}
				}
			}
			for _, sub := range payload.ConfigCommand.SubscriptionsToDelete {
				zlog.Debug().Msgf("Subscription to delete: %v", sub)
				zlog.Debug().Msgf("Delete routes on all other nodes for subscription: %v", sub)
				err := s.routeService.DeleteRoute(ctx, routes.Route{
					SourceNodeID: routes.AllNodesID,
					DestNodeID:   registeredNodeID,
					Component0:   sub.Component_0,
					Component1:   sub.Component_1,
					Component2:   sub.Component_2,
					ComponentID:  sub.Id,
				})
				if err != nil {
					zlog.Error().Msgf("Error deleting route: %v", err)
				}
			}
			continue
		case *controllerapi.ControlMessage_DeregisterNodeRequest:
			// Handle deregistration logic
			if payload.DeregisterNodeRequest.Node != nil {
				nodeID := payload.DeregisterNodeRequest.Node.Id
				zlog.Info().Msgf("Deregister node with ID: %v", nodeID)
				// Update the node status to not connected
				s.nodeCommandHandler.UpdateConnectionStatus(ctx, nodeID, nodecontrol.NodeStatusNotConnected)

				err := s.nodeCommandHandler.RemoveStream(ctx, nodeID)
				if err != nil {
					zlog.Error().Msgf("Error removing stream for node %s: %v", nodeID, err)
				}

				if err = s.nodeCommandHandler.SendMessage(ctx, nodeID, &controllerapi.ControlMessage{
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
			s.nodeCommandHandler.ResponseReceived(ctx, registeredNodeID, msg)
			continue
		case *controllerapi.ControlMessage_ConnectionListResponse:
			zlog.Debug().Msgf("Received ConnectionListResponse: %v", payload.ConnectionListResponse)
			s.nodeCommandHandler.ResponseReceived(ctx, registeredNodeID, msg)
		case *controllerapi.ControlMessage_SubscriptionListResponse:
			zlog.Debug().Msgf(
				"Received SubscriptionListResponse: %v", payload.SubscriptionListResponse)
			s.nodeCommandHandler.ResponseReceived(ctx, registeredNodeID, msg)

		case *controllerapi.ControlMessage_CreateChannelRequest:
			zlog.Debug().Msgf(
				"Received CreateChannelRequest: %v", payload.CreateChannelRequest,
			)
			resp, err := s.groupservice.CreateChannel(
				ctx,
				&controlplaneApi.CreateChannelRequest{
					Moderators: payload.CreateChannelRequest.Moderators,
				},
				&controlplaneApi.NodeEntry{},
			)
			if err != nil {
				zlog.Error().Msgf("Error creating channel: %v", err)
				return err
			}

			ackMsg := &controllerapi.ControlMessage{
				MessageId: uuid.NewString(),
				Payload: &controllerapi.ControlMessage_Ack{
					Ack: &controllerapi.Ack{
						Success:           true,
						OriginalMessageId: msg.MessageId,
					},
				},
			}
			if err := s.nodeCommandHandler.SendMessage(ctx, registeredNodeID, ackMsg); err != nil {
				zlog.Error().Msgf("Error sending CreateChannelResponse: %v", err)
				return err
			}
			zlog.Info().Msgf("Channel created successfully: %s", resp.ChannelName)

		case *controllerapi.ControlMessage_DeleteChannelRequest:
			zlog.Debug().Msgf(
				"Received DeleteChannelRequest: %v", payload.DeleteChannelRequest,
			)
			resp, err := s.groupservice.DeleteChannel(
				ctx,
				payload.DeleteChannelRequest,
				&controlplaneApi.NodeEntry{},
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
			if err := s.nodeCommandHandler.SendMessage(ctx, registeredNodeID, ackMsg); err != nil {
				zlog.Error().Msgf("Error sending DeleteChannelResponse: %v", err)
				return err
			}
			zlog.Info().Msg("Channel deleted successfully")

		case *controllerapi.ControlMessage_AddParticipantRequest:
			zlog.Debug().Msgf(
				"Received AddParticipantRequest: %v", payload.AddParticipantRequest,
			)
			resp, err := s.groupservice.AddParticipant(
				ctx,
				payload.AddParticipantRequest,
				&controlplaneApi.NodeEntry{},
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
			if err := s.nodeCommandHandler.SendMessage(ctx, registeredNodeID, ackMsg); err != nil {
				zlog.Error().Msgf("Error sending AddParticipantResponse: %v", err)
				return err
			}
			zlog.Info().Msgf(
				"Participant added successfully: %s", payload.AddParticipantRequest.ParticipantName)

		case *controllerapi.ControlMessage_DeleteParticipantRequest:
			zlog.Debug().Msgf(
				"Received DeleteParticipantRequest: %v", payload.DeleteParticipantRequest,
			)
			resp, err := s.groupservice.DeleteParticipant(
				ctx,
				payload.DeleteParticipantRequest,
				&controlplaneApi.NodeEntry{},
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
			if err := s.nodeCommandHandler.SendMessage(ctx, registeredNodeID, ackMsg); err != nil {
				zlog.Error().Msgf("Error sending DeleteParticipantResponse: %v", err)
				return err
			}
			zlog.Info().Msgf(
				"Participant deleted successfully: %s",
				payload.DeleteParticipantRequest.ParticipantName,
			)

		case *controllerapi.ControlMessage_ListChannelRequest:
			zlog.Debug().Msgf(
				"Received ListChannelRequest: %v", payload.ListChannelRequest,
			)
			resp, err := s.groupservice.ListChannels(
				ctx,
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
			if err := s.nodeCommandHandler.SendMessage(ctx, registeredNodeID, ackMsg); err != nil {
				zlog.Error().Msgf("Error sending ListChannelResponse: %v", err)
				return err
			}
			zlog.Info().Msg("Channels listed successfully")

		case *controllerapi.ControlMessage_ListParticipantsRequest:
			zlog.Debug().Msgf(
				"Received ListParticipantsRequest: %v", payload.ListParticipantsRequest,
			)
			resp, err := s.groupservice.ListParticipants(
				ctx,
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
			if err := s.nodeCommandHandler.SendMessage(ctx, registeredNodeID, ackMsg); err != nil {
				zlog.Error().Msgf("Error sending ListParticipantsResponse: %v", err)
				return err
			}
			zlog.Info().Msg("Participants listed successfully")

		default:
			zlog.Debug().Msgf("Invalid payload received: %s : %v", msg.MessageId, msg.Payload)
		}
	}
}
