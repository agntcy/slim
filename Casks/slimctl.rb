cask "slimctl" do
  version "3.4.3"

  if Hardware::CPU.intel?
    sha256 "21f742a84d830b6de9d40916d07664fc38e9ce814b105716083629944de6e3c2"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.4.3/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "7770fff06ef660edbd4fbad992cea35755780083e95ba5b05a06f0e360f3583f"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.4.3/slimctl-darwin-arm64.tar.gz"
  end

  name "slimctl"
  desc "A CLI tool for managing SLIM Devices"
  homepage "https://github.com/agntcy/slim"

  binary "slimctl"

  postflight_steps do
    set_permissions "slimctl", "+x", recursive: false
    run "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "{{staged_path}}/slimctl"]
  end
end
