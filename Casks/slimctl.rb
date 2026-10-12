cask "slimctl" do
  version "3.3.0"

  if Hardware::CPU.intel?
    sha256 "701b50a8ea58c6c34d354d5f120b3cf3177c32c9a427023891476ac3908458fe"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.3.0/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "c0435bc512e150885572f8301ef7125ac7f9d7e34769dbf4a9f4e99341add0b0"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.3.0/slimctl-darwin-arm64.tar.gz"
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
