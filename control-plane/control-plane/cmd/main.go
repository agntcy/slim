package main

import (
	"context"
	"crypto/tls"
	"flag"
	"fmt"
	"net"
	"sync"

	"github.com/rs/zerolog"
	"github.com/spiffe/go-spiffe/v2/spiffegrpc/grpccredentials"
	"google.golang.org/grpc"
	"google.golang.org/grpc/credentials"
	"google.golang.org/grpc/peer"

	southboundApi "github.com/agntcy/slim/control-plane/common/proto/controller/v1"
	controlplaneApi "github.com/agntcy/slim/control-plane/common/proto/controlplane/v1"
	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
	"github.com/agntcy/slim/control-plane/control-plane/internal/db"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/groupservice"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nbapiservice"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/nodecontrol"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/routes"
	"github.com/agntcy/slim/control-plane/control-plane/internal/services/sbapiservice"
	"github.com/agntcy/slim/control-plane/control-plane/internal/util"
)

func main() {
	configFile := flag.String("config", "", "configuration file")
	flag.Parse()
	config := config.DefaultConfig().OverrideFromFile(*configFile).OverrideFromEnv().Validate()
	ctx := util.GetContextWithLogger(context.Background(), config.LogConfig)
	zlog := zerolog.Ctx(ctx)

	dbService, err := db.NewSQLiteDBService(config.Database.FilePath)
	if err != nil {
		zlog.Fatal().Msgf("failed to initialize database service: %v", err)
	}
	cmdHandler := nodecontrol.DefaultNodeCommandHandler()
	nodeService := nbapiservice.NewNodeService(dbService, cmdHandler)

	groupService := groupservice.NewGroupService(dbService, cmdHandler)

	routeService := routes.NewRouteService(dbService, cmdHandler, config.Reconciler)
	err = routeService.Start(ctx)
	if err != nil {
		zlog.Fatal().Msgf("failed to start route service: %v", err)
	}

	// wait for go processes to exit
	var wg sync.WaitGroup

	wg.Add(1)
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
		zlog.Info().Msgf("Northbound API Service is listening on %s", lis.Addr())
		err = grpcServer.Serve(lis)
		if err != nil {
			zlog.Fatal().Msgf("failed to serve: %v", err)
		}
		wg.Done()
	}()

	wg.Add(1)
	go func() {
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

		// Add TLS debug interceptors
		opts = append(opts,
			grpc.UnaryInterceptor(tlsDebugUnaryInterceptor),
			grpc.StreamInterceptor(tlsDebugStreamInterceptor),
		)

		sbGrpcServer := grpc.NewServer(opts...)
		sbAPISvc := sbapiservice.NewSBAPIService(config.Southbound, config.LogConfig, dbService, cmdHandler,
			routeService, groupService)
		southboundApi.RegisterControllerServiceServer(sbGrpcServer, sbAPISvc)

		sbListeningAddress := fmt.Sprintf("%s:%s", config.Southbound.HTTPHost, config.Southbound.HTTPPort)
		lisSB, err := net.Listen("tcp", sbListeningAddress)
		if err != nil {
			zlog.Fatal().Msgf("failed to listen: %v", err)
		}
		zlog.Info().Msgf("Southbound API Service is Listening on %s", lisSB.Addr())
		err = sbGrpcServer.Serve(lisSB)
		if err != nil {
			zlog.Fatal().Msgf("failed to serve: %v", err)
		}
		wg.Done()
	}()

	wg.Wait()
}

// Add this to your main.go
func tlsDebugUnaryInterceptor(ctx context.Context, req interface{}, info *grpc.UnaryServerInfo,
	handler grpc.UnaryHandler) (interface{}, error) {
	zlog := zerolog.Ctx(ctx)

	sid, ok := grpccredentials.PeerIDFromContext(ctx)
	if ok {
		trustDomain := sid.TrustDomain().String()
		zlog.Debug().Msgf("TLS Debug Unary Interceptor: trustDomain: %s", trustDomain)
	}

	// Extract TLS connection info
	if peer, ok := peer.FromContext(ctx); ok {

		sid, ok := grpccredentials.PeerIDFromPeer(peer)
		if ok {
			trustDomain := sid.TrustDomain().String()
			zlog.Debug().Msgf("TLS Debug Stream Interceptor: trustDomain: %s", trustDomain)
		}

		if tlsInfo, ok := peer.AuthInfo.(credentials.TLSInfo); ok {
			zlog.Debug().
				Str("method", info.FullMethod).
				Str("remote_addr", peer.Addr.String()).
				Str("tls_version", getTLSVersion(tlsInfo.State.Version)).
				Str("cipher_suite", tls.CipherSuiteName(tlsInfo.State.CipherSuite)).
				Bool("handshake_complete", tlsInfo.State.HandshakeComplete).
				Msg("TLS connection info")

			// Log client certificate info if present
			if len(tlsInfo.State.PeerCertificates) > 0 {
				clientCert := tlsInfo.State.PeerCertificates[0]
				zlog.Debug().
					Str("method", info.FullMethod).
					Str("client_subject", clientCert.Subject.String()).
					Str("client_issuer", clientCert.Issuer.String()).
					Str("client_serial", clientCert.SerialNumber.String()).
					Msg("Client certificate authenticated")
			} else {
				zlog.Debug().
					Str("method", info.FullMethod).
					Msg("No client certificate provided")
			}
		}
	}

	return handler(ctx, req)
}

func tlsDebugStreamInterceptor(srv interface{}, ss grpc.ServerStream, info *grpc.StreamServerInfo,
	handler grpc.StreamHandler) error {
	ctx := ss.Context()
	zlog := zerolog.Ctx(ctx)

	if peer, ok := peer.FromContext(ctx); ok {
		sid, ok := grpccredentials.PeerIDFromPeer(peer)
		if ok {
			trustDomain := sid.TrustDomain().String()
			zlog.Debug().Msgf("TLS Debug Stream Interceptor: trustDomain: %s", trustDomain)
		}

		if tlsInfo, ok := peer.AuthInfo.(credentials.TLSInfo); ok {
			zlog.Debug().
				Str("stream", info.FullMethod).
				Str("remote_addr", peer.Addr.String()).
				Bool("handshake_complete", tlsInfo.State.HandshakeComplete).
				Msg("TLS stream connection")
		}
	}

	return handler(srv, ss)
}

func getTLSVersion(version uint16) string {
	switch version {
	case tls.VersionTLS10:
		return "TLS 1.0"
	case tls.VersionTLS11:
		return "TLS 1.1"
	case tls.VersionTLS12:
		return "TLS 1.2"
	case tls.VersionTLS13:
		return "TLS 1.3"
	default:
		return fmt.Sprintf("Unknown (0x%04x)", version)
	}
}
