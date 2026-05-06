package util

import (
	"context"
	"crypto/tls"
	"crypto/x509"
	"fmt"
	"os"

	"github.com/rs/zerolog"
	"github.com/spiffe/go-spiffe/v2/spiffetls/tlsconfig"
	"github.com/spiffe/go-spiffe/v2/workloadapi"
	"google.golang.org/grpc/credentials"

	"github.com/agntcy/slim/control-plane/control-plane/internal/config"
)

func LoadCertificates(ctx context.Context, apiConfig config.APIConfig) (credentials.TransportCredentials, error) {
	zlog := zerolog.Ctx(ctx)

	var tlsConfig *tls.Config
	cfg := apiConfig.TLS
	if cfg.UseSpiffe {
		cl := workloadapi.WithClientOptions(workloadapi.WithAddr(apiConfig.Spire.SocketPath))
		source, err := workloadapi.NewX509Source(ctx, cl)
		if err != nil {
			return nil, fmt.Errorf("failed to create X.509 source using SPIRE Workload API: %w", err)
		}
		bundleSource, err := workloadapi.NewBundleSource(ctx, cl)
		if err != nil {
			return nil, fmt.Errorf("failed to create X.509 bundle source using SPIRE Workload API: %w", err)
		}
		tlsConfig = tlsconfig.MTLSServerConfig(source, bundleSource, tlsconfig.AuthorizeAny())
	} else {
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
		tlsConfig = &tls.Config{
			Certificates: []tls.Certificate{cert},
			ClientCAs:    clientCAPool,
			ClientAuth:   clientAuth,
			MinVersion:   tls.VersionTLS12,
			MaxVersion:   tls.VersionTLS13,
		}
	}

	// Log peer certs after verification. Use VerifyConnection (not VerifyPeerCertificate) so logging
	// still runs when TLS session resumption is used; VerifyPeerCertificate can be skipped on resume (gosec G123).
	tlsConfig.VerifyConnection = func(state tls.ConnectionState) error {
		certs := state.PeerCertificates
		zlog.Debug().Int("cert_count", len(certs)).Msg("Received client certificates")

		for i, cert := range certs {
			zlog.Debug().
				Int("cert_index", i).
				Str("subject", cert.Subject.String()).
				Str("issuer", cert.Issuer.String()).
				Str("serial", cert.SerialNumber.String()).
				Time("not_before", cert.NotBefore).
				Time("not_after", cert.NotAfter).
				Msg("Client certificate details")
		}

		zlog.Debug().Int("verified_chain_count", len(state.VerifiedChains)).Msg("Verified certificate chains")

		return nil
	}
	return credentials.NewTLS(tlsConfig), nil
}
