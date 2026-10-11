cask "slimctl" do
  version "3.2.1"

  if Hardware::CPU.intel?
    sha256 "d92c4d3f4524e6b62451f0b46bd25c0da52efe6c7cf1e2fd34beb07c819363ea"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.2.1/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "fc4230a62cebda05754fe8d356e345f47086ff9c025dd8f5eea2a994cbfc553b"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.2.1/slimctl-darwin-arm64.tar.gz"
  end

  name "slimctl"
  desc "A CLI tool for managing SLIM Devices"
  homepage "https://github.com/agntcy/slim"

  binary "slimctl"

  postflight do
    system "chmod", "+x", "#{staged_path}/slimctl"
    system "/usr/bin/xattr", "-dr", "com.apple.quarantine", "#{staged_path}/slimctl" if MacOS.version >= :catalina
  end
end
