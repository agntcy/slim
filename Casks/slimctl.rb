cask "slimctl" do
  version "1.4.0-rc.0"

  if Hardware::CPU.intel?
    sha256 "5ebba9d7fd22216d91b0252b7f88a04e69ede8994aded792452b98c6a9e59585"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v1.4.0-rc.0/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "74f525463f28546b252dc006c64db3f65905dac0a3969a48d86d799067ed33d4"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v1.4.0-rc.0/slimctl-darwin-arm64.tar.gz"
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
