cask "slimctl" do
  version "3.1.0"

  if Hardware::CPU.intel?
    sha256 "61d3a8b63dab6e789949214778763282cf4ac99d437db4c58455b1db9e7ef771"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.1.0/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "881f4c397759e16c7c1c84250adeaf6e503b10fb55a504552bf39ec9d6b4e01d"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.1.0/slimctl-darwin-arm64.tar.gz"
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
