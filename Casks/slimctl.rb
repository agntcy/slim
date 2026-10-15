cask "slimctl" do
  version "3.4.1"

  if Hardware::CPU.intel?
    sha256 "b4e7663b12ef731bfb8f1fbbd63cdee7ad42b4b7c5cd496d31d7cd90c2a0033b"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.4.1/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "ba388e29d8b9c77830b14191d2caa75da0e9de8d731ab5018a0acc5ee9663612"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.4.1/slimctl-darwin-arm64.tar.gz"
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
