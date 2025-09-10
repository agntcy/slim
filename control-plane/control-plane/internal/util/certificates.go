package util

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"os"

	"github.com/spiffe/go-spiffe/v2/spiffetls/tlsconfig"
	"github.com/spiffe/go-spiffe/v2/workloadapi"
	"google.golang.org/grpc/credentials"

	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
)

func LoadCertificates(ctx context.Context, apiConfig config.APIConfig) (credentials.TransportCredentials, error) {

	cfg := apiConfig.TLS
	if cfg.UseSpiffe {
		source, err := workloadapi.NewX509Source(ctx,
			workloadapi.WithClientOptions(workloadapi.WithAddr(apiConfig.Spire.SocketPath)))
		if err != nil {
			return nil, fmt.Errorf("failed to create X.509 source using SPIRE Workload API: %w", err)
		}
		tlsConfig := tlsconfig.MTLSServerConfig(source, source, tlsconfig.AuthorizeAny())
		return credentials.NewTLS(tlsConfig), nil
	}

	cert, err := tls.LoadX509KeyPair(cfg.CertFile, cfg.KeyFile)
	if err != nil {
		return nil, fmt.Errorf("failed to load server cert/key: %w", err)
	}

	caCert, err := os.ReadFile(cfg.CAFile)
	if err != nil {
		return nil, fmt.Errorf("failed to read client CA cert: %w", err)
	}
	clientCAPool := x509.NewCertPool()
	if !clientCAPool.AppendCertsFromPEM(caCert) {
		return nil, fmt.Errorf("failed to append client CA cert")
	}

	clientAuth := tls.NoClientCert
	if cfg.CAFile != "" {
		clientAuth = tls.RequireAndVerifyClientCert
	}
	tlsConfig := &tls.Config{
		Certificates: []tls.Certificate{cert},
		ClientCAs:    clientCAPool,
		ClientAuth:   clientAuth,
		MinVersion:   tls.VersionTLS12,
		MaxVersion:   tls.VersionTLS13,
	}
	return credentials.NewTLS(tlsConfig), nil
}
