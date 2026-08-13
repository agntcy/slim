cask "slimctl" do
  version "2.3.0"

  if Hardware::CPU.intel?
    sha256 "5e04efba5c674e0a8d5d5c83dd26ff726c3f07421b4a0ecd9c388ad5a53086f3"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v2.3.0/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "19e5938c63c8049489c2b6c69da2bac163cd7f2dfa883cb0036aee51ce9ef98a"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v2.3.0/slimctl-darwin-arm64.tar.gz"
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
