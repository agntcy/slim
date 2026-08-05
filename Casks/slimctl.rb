cask "slimctl" do
  version "2.0.0"

  if Hardware::CPU.intel?
    sha256 "22b03eae72ec2ad2c33f224fdb2244602df59f19f5535d00dafeedddcc374f0d"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "52e28b3c8f3492f741b258faf7c4db76ec693a2f8eaa5eba9d5aae5519793790"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0/slimctl-darwin-arm64.tar.gz"
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
