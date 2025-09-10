package main

import (
	"context"
	"flag"
	"fmt"
	"net"

	"github.com/rs/zerolog"
	"google.golang.org/grpc"

	"github.com/agntcy/slim/control-plane/control-plane/internal/util"

	southboundApi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/groupservice"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nbapiservice"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/sbapiservice"
)

func main() {
	configFile := flag.String("config", "", "configuration file")
	flag.Parse()
	config := config.DefaultConfig().OverrideFromFile(*configFile).OverrideFromEnv().Validate()
	ctx := util.GetContextWithLogger(context.Background(), config.LogConfig)
	zlog := zerolog.Ctx(ctx)

	dbService := db.NewInMemoryDBService()
	cmdHandler := nodecontrol.DefaultNodeCommandHandler()
	nodeService := nbapiservice.NewNodeService(dbService, cmdHandler)
	routeService := nbapiservice.NewRouteService(cmdHandler)
	groupService := groupservice.NewGroupService(dbService)
	registrationService := nbapiservice.NewNodeRegistrationService(dbService, cmdHandler)

	go func() {
		cpServer := nbapiservice.NewNorthboundAPIServer(config.Northbound, config.LogConfig,
			nodeService, routeService, groupService)
		var opts []grpc.ServerOption
		grpcServer := grpc.NewServer(opts...)
		controlplaneApi.RegisterControlPlaneServiceServer(grpcServer, cpServer)

		listeningAddress := fmt.Sprintf("%s:%s", config.Northbound.HTTPHost, config.Northbound.HTTPPort)
		lis, err := net.Listen("tcp", listeningAddress)
		if err != nil {
			zlog.Fatal().Msgf("failed to listen: %v", err)
		}
		zlog.Info().Msgf("Northbound API Service is listening on %s\n", lis.Addr())
		err = grpcServer.Serve(lis)
		if err != nil {
			zlog.Fatal().Msgf("failed to serve: %v", err)
		}
	}()

	var opts []grpc.ServerOption
	if config.Southbound.TLS != nil {
		creds, err := util.LoadCertificates(ctx, config.Southbound)
		if err != nil {
			zlog.Fatal().Msgf("TLS setup error: %v", err)
		}
		if creds != nil {
			opts = append(opts, grpc.Creds(creds))
		}
	}

	sbGrpcServer := grpc.NewServer(opts...)
	sbAPISvc := sbapiservice.NewSBAPIService(config.Southbound, config.LogConfig, dbService, cmdHandler,
		[]nodecontrol.NodeRegistrationHandler{registrationService}, groupService)
	southboundApi.RegisterControllerServiceServer(sbGrpcServer, sbAPISvc)

	sbListeningAddress := fmt.Sprintf("%s:%s", config.Southbound.HTTPHost, config.Southbound.HTTPPort)
	lisSB, err := net.Listen("tcp", sbListeningAddress)
	if err != nil {
		zlog.Fatal().Msgf("failed to listen: %v", err)
	}
	zlog.Info().Msgf("Southbound API Service is Listening on %s\n", lisSB.Addr())
	err = sbGrpcServer.Serve(lisSB)
	if err != nil {
		zlog.Fatal().Msgf("failed to serve: %v", err)
	}
}
