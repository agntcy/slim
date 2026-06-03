cask "slimctl" do
  version "2.0.0-alpha.0"

  if Hardware::CPU.intel?
    sha256 "0beed7e9e78580ca43416d281bafaf2fa64cf6933acab487938ae5e58ab46a62"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0-alpha.0/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "8abbf67963dab9a463577adc7fec342ab04e30ed1a6b01375affeaa8dd424be5"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0-alpha.0/slimctl-darwin-arm64.tar.gz"
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
