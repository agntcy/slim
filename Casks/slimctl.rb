cask "slimctl" do
  version "1.3.0-rc.1"

  if Hardware::CPU.intel?
    sha256 "140104a0f2e95d62284925ed169d4b0bc3406823aa714ea827eae00d3c446c64"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v1.3.0-rc.1/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "7e704dd972053b1ae9f1dbb7e61a24eafcd996e96584c39fdfabc77e2bef34ed"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v1.3.0-rc.1/slimctl-darwin-arm64.tar.gz"
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
