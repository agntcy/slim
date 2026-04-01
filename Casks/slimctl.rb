cask "slimctl" do
  version "1.3.0"

  if Hardware::CPU.intel?
    sha256 "b8cfc054299131e95605a05fa8afe14845d4dea41b4fa85f7baf4e6f2e5b75d4"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v1.3.0/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "f0afca0ee9a730f0e8c115d5c07a745fad2cfaf79093c38abd604f42dfbae9f5"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v1.3.0/slimctl-darwin-arm64.tar.gz"
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
