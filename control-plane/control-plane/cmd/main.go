package main

import (
	"flag"
	"fmt"
	"log"
	"net"

	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nbapiservice"
	"google.golang.org/grpc"
)

func main() {
	configFile := flag.String("config", "", "configuration file")
	flag.Parse()
	config := config.DefaultConfig().OverrideFromFile(*configFile).OverrideFromEnv().Validate()

	var opts []grpc.ServerOption
	grpcServer := grpc.NewServer(opts...)
	dbService := db.NewInMemoryDBService()
	nodeService := nbapiservice.NewNodeService(dbService)
	routeService := nbapiservice.NewRouteService()
	configService := nbapiservice.NewConfigService()
	cpServer := nbapiservice.NewNorthboundAPIServer(nodeService, routeService, configService)
	controlplaneApi.RegisterControlPlaneServiceServer(grpcServer, cpServer)
	listeningAddress := fmt.Sprintf("%s:%s", config.HttpHost, config.HttpPort)
	lis, err := net.Listen("tcp", listeningAddress)
	if err != nil {
		log.Fatalf("failed to listen: %v", err)
	}
	grpcServer.Serve(lis)
}
