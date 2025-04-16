// Copyright AGNTCY Contributors (https://github.com/agntcy)
// SPDX-License-Identifier: Apache-2.0

package controller

import (
	"context"
	"fmt"
	"time"

	controllerv1 "github.com/agntcy/agp/control-plane/internal/proto/controller/v1"

	"go.uber.org/zap"
	"google.golang.org/grpc"
	"google.golang.org/grpc/codes"
	"google.golang.org/grpc/metadata"
	"google.golang.org/grpc/peer"
	"google.golang.org/grpc/status"
)

const bufSize = 1024 * 1024

type ControllerService struct {
	controlTx chan<- ControlCommand
	logger    *zap.Logger
}

func NewControllerService(controlTx chan<- ControlCommand, logger *zap.Logger) *ControllerService {
	return &ControllerService{
		controlTx: controlTx,
		logger:    logger,
	}
}

func (cs *ControllerService) OpenChannel(stream controllerv1.ControllerService_OpenChannelServer) error {
	if md, ok := metadata.FromIncomingContext(stream.Context()); ok {
		cs.logger.Info("OpenChannel metadata", zap.Any("metadata", md))
	}
	if p, ok := peer.FromContext(stream.Context()); ok {
		cs.logger.Info("OpenChannel peer", zap.String("addr", p.Addr.String()))
	}

	for {
		cmd, err := stream.Recv()
		if err != nil {
			cs.logger.Error("Error receiving command", zap.Error(err))
			return err
		}
		cs.logger.Info("Received command", zap.Any("command", cmd))

		var internalCmd ControlCommand
		switch x := cmd.CommandType.(type) {
		case *controllerv1.Command_Subscribe:
			sub := x.Subscribe
			internalCmd = SubscribeCommand{
				Topic:    sub.Topic,
				ClientID: sub.ClientId,
			}
		case *controllerv1.Command_Unsubscribe:
			unsub := x.Unsubscribe
			internalCmd = UnsubscribeCommand{
				SubscriptionID: unsub.SubscriptionId,
			}
		default:
			cs.logger.Error("Unknown command type", zap.Any("commandType", cmd.CommandType))
			return status.Errorf(codes.InvalidArgument, "unknown command type")
		}

		if err := cs.forwardCommand(internalCmd); err != nil {
			cs.logger.Error("Failed to forward command", zap.Error(err))
			return status.Errorf(codes.Internal, "failed to process command: %v", err)
		}

		ack := &controllerv1.Command{
			CommandType: cmd.CommandType,
			Metadata:    map[string]string{},
		}
		if err := stream.Send(ack); err != nil {
			cs.logger.Error("Failed to send ack", zap.Error(err))
			return err
		}
	}
}

func (cs *ControllerService) forwardCommand(cmd ControlCommand) error {
	select {
	case cs.controlTx <- cmd:
		cs.logger.Info("Forwarded control command", zap.Any("command", cmd))
		return nil
	case <-time.After(5 * time.Second):
		return fmt.Errorf("timeout forwarding control command")
	}
}

func ConnectControllerService(ctx context.Context, addr string, logger *zap.Logger) (controllerv1.ControllerServiceClient, *grpc.ClientConn, error) {
	conn, err := grpc.DialContext(ctx, addr, grpc.WithInsecure(), grpc.WithBlock())
	if err != nil {
		return nil, nil, fmt.Errorf("failed to dial controller service: %w", err)
	}
	client := controllerv1.NewControllerServiceClient(conn)
	logger.Info("Connected to controller service", zap.String("addr", addr))
	return client, conn, nil
}
