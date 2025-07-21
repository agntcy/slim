package main

import (
	"flag"
	"fmt"
	"log"
	"net"

	southboundApi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/messagingservice"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nbapiservice"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/sbapiservice"
	"google.golang.org/grpc"
)

func main() {
	configFile := flag.String("config", "", "configuration file")
	flag.Parse()
	config := config.DefaultConfig().OverrideFromFile(*configFile).OverrideFromEnv().Validate()

	var opts []grpc.ServerOption

	dbService := db.NewInMemoryDBService()
	messagingService := messagingservice.NewMessagingService()
	nodeService := nbapiservice.NewNodeService(dbService)
	routeService := nbapiservice.NewRouteService(messagingService)
	configService := nbapiservice.NewConfigService()

	go func() {
		cpServer := nbapiservice.NewNorthboundAPIServer(nodeService, routeService, configService)
		grpcServer := grpc.NewServer(opts...)
		controlplaneApi.RegisterControlPlaneServiceServer(grpcServer, cpServer)

		listeningAddress := fmt.Sprintf("%s:%s", config.Northbound.HttpHost, config.Northbound.HttpPort)
		lis, err := net.Listen("tcp", listeningAddress)
		if err != nil {
			log.Fatalf("failed to listen: %v", err)
		}
		fmt.Printf("Northbound API Service is listening on %s\n", lis.Addr())
		grpcServer.Serve(lis)
	}()

	sbGrpcServer := grpc.NewServer(opts...)
	sbApiSvc := sbapiservice.NewSBAPIService(dbService, messagingService)
	southboundApi.RegisterControllerServiceServer(sbGrpcServer, sbApiSvc)

	sbListeningAddress := fmt.Sprintf("%s:%s", config.Southbound.HttpHost, config.Southbound.HttpPort)
	lisSB, err := net.Listen("tcp", sbListeningAddress)
	if err != nil {
		log.Fatalf("failed to listen: %v", err)
	}
	fmt.Printf("Southbound API Service is Listening on %s\n", lisSB.Addr())
	sbGrpcServer.Serve(lisSB)
}
