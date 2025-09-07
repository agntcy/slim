package main

import (
	"context"
	"flag"
	"fmt"
	"log"
	"net"

	"github.com/agntcy/slim/control-plane/control-plane/internal/util"
	"google.golang.org/grpc"

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

	ctx := context.Background()

	dbService := db.NewInMemoryDBService()
	cmdHandler := nodecontrol.DefaultNodeCommandHandler()
	nodeService := nbapiservice.NewNodeService(dbService, cmdHandler)
	routeService := nbapiservice.NewRouteService(cmdHandler)
	groupService := groupservice.NewGroupService(dbService)
	registrationService := nbapiservice.NewNodeRegistrationService(dbService, cmdHandler)

	go func() {
		cpServer := nbapiservice.NewNorthboundAPIServer(config.Northbound, nodeService, routeService, groupService)
		var opts []grpc.ServerOption
		grpcServer := grpc.NewServer(opts...)
		controlplaneApi.RegisterControlPlaneServiceServer(grpcServer, cpServer)

		listeningAddress := fmt.Sprintf("%s:%s", config.Northbound.HTTPHost, config.Northbound.HTTPPort)
		lis, err := net.Listen("tcp", listeningAddress)
		if err != nil {
			log.Fatalf("failed to listen: %v", err)
		}
		fmt.Printf("Northbound API Service is listening on %s\n", lis.Addr())
		err = grpcServer.Serve(lis)
		if err != nil {
			log.Fatalf("failed to serve: %v", err)
		}
	}()

	var opts []grpc.ServerOption
	if config.Southbound.TLS != nil {
		creds, err := util.LoadCertificates(ctx, config.Southbound)
		if err != nil {
			log.Fatalf("TLS setup error: %v", err)
		}
		if creds != nil {
			opts = append(opts, grpc.Creds(creds))
		}
	}

	sbGrpcServer := grpc.NewServer(opts...)
	sbAPISvc := sbapiservice.NewSBAPIService(config.Southbound, dbService, cmdHandler,
		[]nodecontrol.NodeRegistrationHandler{registrationService}, groupService)
	southboundApi.RegisterControllerServiceServer(sbGrpcServer, sbAPISvc)

	sbListeningAddress := fmt.Sprintf("%s:%s", config.Southbound.HTTPHost, config.Southbound.HTTPPort)
	lisSB, err := net.Listen("tcp", sbListeningAddress)
	if err != nil {
		log.Fatalf("failed to listen: %v", err)
	}
	fmt.Printf("Southbound API Service is Listening on %s\n", lisSB.Addr())
	err = sbGrpcServer.Serve(lisSB)
	if err != nil {
		log.Fatalf("failed to serve: %v", err)
	}
}
