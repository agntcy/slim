cask "slimctl" do
  version "1.4.0"

  if Hardware::CPU.intel?
    sha256 "bcc3bfffac869bddecdbbb97b991ac8a79803d62b3124655f698d7232ec08867"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v1.4.0/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "36b17520c07eab87371941c9034725050e2dd5bf79e7f8cf1eaa2eddd1a52614"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v1.4.0/slimctl-darwin-arm64.tar.gz"
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
