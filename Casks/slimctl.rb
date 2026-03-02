cask "slimctl" do
  version "1.2.0"

  if Hardware::CPU.intel?
    sha256 "d8a4ae8b1be264ad67c72d5703b785a7924f8fd2292d92310535e8893d58a864"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "e2060c0a58f240a9c3b52940a45b5f543ac9ffbb7cf0762d753e111deb492f85"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v1.2.0/slimctl-darwin-arm64.tar.gz"
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
