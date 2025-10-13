package config

import (
	"fmt"
	"strconv"
	"time"

	"github.com/spf13/cobra"

	"github.com/agntcy/slim/control-plane/slimctl/internal/cfg"
)

func newSetConfigCmd(conf *cfg.ConfigData) *cobra.Command {
	cmd := &cobra.Command{
		Use:   "set",
		Short: "Set and save configuration values",
		Long:  `Set and save configuration values`,
	}

	cmd.AddCommand(newSetBasicAuthCredsCmd(conf))
	cmd.AddCommand(newSetServerCmd(conf))
	cmd.AddCommand(newSetTimeoutCmd(conf))
	cmd.AddCommand(newSetTLSCACmd(conf))
	cmd.AddCommand(newSetTLSCertFileCmd(conf))
	cmd.AddCommand(newSetTLSKeyFileCmd(conf))
	cmd.AddCommand(newSetTLSInsecureCmd(conf))

	return cmd
}

func newSetBasicAuthCredsCmd(conf *cfg.ConfigData) *cobra.Command {
	return &cobra.Command{
		Use:   "basic-auth-creds",
		Short: "Set basic auth credentials",
		Long:  "Set basic auth credentials",
		Args:  cobra.ExactArgs(1),
		RunE: func(_ *cobra.Command, args []string) error {
			creds := args[0]
			if creds == "" {
				return fmt.Errorf("credentials are required")
			}

			appConf, err := cfg.LoadConfig(conf.Fs)
			if err != nil {
				return err
			}
			appConf.CommonOpts.BasicAuthCredentials = creds
			conf.AppConfig = appConf
			err = conf.SaveConfig()
			if err != nil {
				return err
			}
			return nil
		},
	}
}

func newSetServerCmd(conf *cfg.ConfigData) *cobra.Command {
	return &cobra.Command{
		Use:   "server",
		Short: "Set server address",
		Long:  "Set server address",
		Args:  cobra.ExactArgs(1),
		RunE: func(_ *cobra.Command, args []string) error {
			server := args[0]
			if server == "" {
				return fmt.Errorf("server address is required")
			}

			appConf, err := cfg.LoadConfig(conf.Fs)
			if err != nil {
				return err
			}
			appConf.CommonOpts.Server = server
			conf.AppConfig = appConf

			err = conf.SaveConfig()
			if err != nil {
				return err
			}
			return nil
		},
	}
}

func newSetTimeoutCmd(conf *cfg.ConfigData) *cobra.Command {
	return &cobra.Command{
		Use:   "timeout",
		Short: "Set request timeout",
		Long:  "Set request timeout",
		Args:  cobra.ExactArgs(1),
		RunE: func(_ *cobra.Command, args []string) error {
			inputTimeout := args[0]
			if inputTimeout == "" {
				return fmt.Errorf("timeout is required")
			}

			appConf, err := cfg.LoadConfig(conf.Fs)
			if err != nil {
				return err
			}
			timeout, err := time.ParseDuration(inputTimeout)
			if err != nil {
				return err
			}
			appConf.CommonOpts.Timeout = timeout
			conf.AppConfig = appConf

			err = conf.SaveConfig()
			if err != nil {
				return err
			}
			return nil
		},
	}
}

func newSetTLSCACmd(conf *cfg.ConfigData) *cobra.Command {
	return &cobra.Command{
		Use:   "tls-ca-file",
		Short: "Set TLS CA file",
		Long:  "Set TLS CA file",
		Args:  cobra.ExactArgs(1),
		RunE: func(_ *cobra.Command, args []string) error {
			tlsCAFile := args[0]
			if tlsCAFile == "" {
				return fmt.Errorf("TLS CA file is required")
			}

			appConf, err := cfg.LoadConfig(conf.Fs)
			if err != nil {
				return err
			}
			appConf.CommonOpts.TLSCAFile = tlsCAFile
			conf.AppConfig = appConf

			err = conf.SaveConfig()
			if err != nil {
				return err
			}
			return nil
		},
	}
}

func newSetTLSCertFileCmd(conf *cfg.ConfigData) *cobra.Command {
	return &cobra.Command{
		Use:   "tls-cert-file",
		Short: "Set TLS certificate file",
		Long:  "Set TLS certificate file",
		Args:  cobra.ExactArgs(1),
		RunE: func(_ *cobra.Command, args []string) error {
			tlsCertFile := args[0]
			if tlsCertFile == "" {
				return fmt.Errorf("TLS certificate file is required")
			}

			appConf, err := cfg.LoadConfig(conf.Fs)
			if err != nil {
				return err
			}
			appConf.CommonOpts.TLSCertFile = tlsCertFile
			conf.AppConfig = appConf

			err = conf.SaveConfig()
			if err != nil {
				return err
			}
			return nil
		},
	}
}

func newSetTLSKeyFileCmd(conf *cfg.ConfigData) *cobra.Command {
	return &cobra.Command{
		Use:   "tls-key-file",
		Short: "Set TLS key file",
		Long:  "Set TLS key file",
		Args:  cobra.ExactArgs(1),
		RunE: func(_ *cobra.Command, args []string) error {
			tlsKeyFile := args[0]
			if tlsKeyFile == "" {
				return fmt.Errorf("TLS key file is required")
			}

			appConf, err := cfg.LoadConfig(conf.Fs)
			if err != nil {
				return err
			}
			appConf.CommonOpts.TLSKeyFile = tlsKeyFile
			conf.AppConfig = appConf

			err = conf.SaveConfig()
			if err != nil {
				return err
			}
			return nil
		},
	}
}

func newSetTLSInsecureCmd(conf *cfg.ConfigData) *cobra.Command {
	return &cobra.Command{
		Use:   "tls-insecure",
		Short: "Set TLS insecure",
		Long:  "Set TLS insecure",
		Args:  cobra.ExactArgs(1),
		RunE: func(_ *cobra.Command, args []string) error {
			inputTLSInsecure := args[0]
			if inputTLSInsecure == "" {
				return fmt.Errorf("TLS insecure is required")
			}

			appConf, err := cfg.LoadConfig(conf.Fs)
			if err != nil {
				return err
			}
			tlsInsecure, err := strconv.ParseBool(inputTLSInsecure)
			if err != nil {
				return err
			}
			appConf.CommonOpts.TLSInsecure = tlsInsecure
			conf.AppConfig = appConf

			err = conf.SaveConfig()
			if err != nil {
				return err
			}
			return nil
		},
	}
}
