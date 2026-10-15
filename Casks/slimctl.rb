cask "slimctl" do
  version "3.4.2"

  if Hardware::CPU.intel?
    sha256 "97eed5e4a6fa34f4f329878fb9c1873a61c30a0f9a7e683ec4f3e67f9e561f35"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.4.2/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "88d35d900bde7308199b9be2d29313c0448a9d3a90e6f41c70611b387eefc4a9"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.4.2/slimctl-darwin-arm64.tar.gz"
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
