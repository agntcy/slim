cask "slimctl" do
  version "1.3.0-rc.0"

  if Hardware::CPU.intel?
    sha256 "90da56131cd00a93c48d905fe0a0292225defc4784c227a0e9498397ac0afe32"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v1.3.0-rc.0/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "f02f322425614a1d55cae635ffd9c6dbf6191582d0eda44da4638b3473890d51"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v1.3.0-rc.0/slimctl-darwin-arm64.tar.gz"
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
