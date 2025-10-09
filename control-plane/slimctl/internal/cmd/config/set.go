package config

import (
	"fmt"
	"strconv"
	"time"

	"github.com/spf13/cobra"
)

func newSetConfigCmd() *cobra.Command {
	cmd := &cobra.Command{
		Use:   "set",
		Short: "Set and save configuration values",
		Long:  `Set and save configuration values`,
	}

	cmd.AddCommand(newSetBasicAuthCredsCmd())
	cmd.AddCommand(newSetServerCmd())
	cmd.AddCommand(newSetTimeoutCmd())
	cmd.AddCommand(newSetTLSCACmd())
	cmd.AddCommand(newSetTLSCertFileCmd())
	cmd.AddCommand(newSetTLSKeyFileCmd())
	cmd.AddCommand(newSetTLSInsecureCmd())

	return cmd
}

func newSetBasicAuthCredsCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "basic-auth-creds",
		Short: "Set basic auth credentials",
		Long:  "Set basic auth credentials",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			creds := args[0]
			if creds == "" {
				return fmt.Errorf("credentials are required")
			}

			conf, err := loadConfig()
			if err != nil {
				return err
			}
			fmt.Print("%v", conf)

			conf.BasicAuthCredentials = creds

			fmt.Print("Updated config: %v", conf)

			err = saveConfig(conf)
			if err != nil {
				return err
			}
			return nil
		},
	}
}

func newSetServerCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "server",
		Short: "Set server address",
		Long:  "Set server address",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			server := args[0]
			if server == "" {
				return fmt.Errorf("server address is required")
			}

			conf, err := loadConfig()
			if err != nil {
				return err
			}
			fmt.Print("%v", conf)

			conf.Server = server

			fmt.Print("Updated config: %v", conf)

			err = saveConfig(conf)
			if err != nil {
				return err
			}
			return nil
		},
	}
}

func newSetTimeoutCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "timeout",
		Short: "Set request timeout",
		Long:  "Set request timeout",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			inputTimeout := args[0]
			if inputTimeout == "" {
				return fmt.Errorf("timeout is required")
			}

			conf, err := loadConfig()
			if err != nil {
				return err
			}
			fmt.Print("%v", conf)

			timeout, err := time.ParseDuration(inputTimeout)
			if err != nil {
				return err
			}
			conf.Timeout = timeout

			fmt.Print("Updated config: %v", conf)

			err = saveConfig(conf)
			if err != nil {
				return err
			}
			return nil
		},
	}
}

func newSetTLSCACmd() *cobra.Command {
	return &cobra.Command{
		Use:   "tls-ca-file",
		Short: "Set TLS CA file",
		Long:  "Set TLS CA file",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			tlsCAFile := args[0]
			if tlsCAFile == "" {
				return fmt.Errorf("TLS CA file is required")
			}

			conf, err := loadConfig()
			if err != nil {
				return err
			}
			fmt.Print("%v", conf)

			conf.TLSCAFile = tlsCAFile

			fmt.Print("Updated config: %v", conf)

			err = saveConfig(conf)
			if err != nil {
				return err
			}
			return nil
		},
	}
}

func newSetTLSCertFileCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "tls-cert-file",
		Short: "Set TLS certificate file",
		Long:  "Set TLS certificate file",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			tlsCertFile := args[0]
			if tlsCertFile == "" {
				return fmt.Errorf("TLS certificate file is required")
			}

			conf, err := loadConfig()
			if err != nil {
				return err
			}
			fmt.Print("%v", conf)

			conf.TLSCertFile = tlsCertFile

			fmt.Print("Updated config: %v", conf)

			err = saveConfig(conf)
			if err != nil {
				return err
			}
			return nil
		},
	}
}

func newSetTLSKeyFileCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "tls-key-file",
		Short: "Set TLS key file",
		Long:  "Set TLS key file",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			tlsKeyFile := args[0]
			if tlsKeyFile == "" {
				return fmt.Errorf("TLS key file is required")
			}

			conf, err := loadConfig()
			if err != nil {
				return err
			}
			fmt.Print("%v", conf)

			conf.TLSKeyFile = tlsKeyFile

			fmt.Print("Updated config: %v", conf)

			err = saveConfig(conf)
			if err != nil {
				return err
			}
			return nil
		},
	}
}

func newSetTLSInsecureCmd() *cobra.Command {
	return &cobra.Command{
		Use:   "tls-insecure",
		Short: "Set TLS insecure",
		Long:  "Set TLS insecure",
		Args:  cobra.ExactArgs(1),
		RunE: func(cmd *cobra.Command, args []string) error {
			inputTLSInsecure := args[0]
			if inputTLSInsecure == "" {
				return fmt.Errorf("TLS insecure is required")
			}

			conf, err := loadConfig()
			if err != nil {
				return err
			}
			fmt.Print("%v", conf)

			tlsInsecure, err := strconv.ParseBool(inputTLSInsecure)
			if err != nil {
				return err
			}
			conf.TLSInsecure = tlsInsecure

			fmt.Print("Updated config: %v", conf)

			err = saveConfig(conf)
			if err != nil {
				return err
			}
			return nil
		},
	}
}
