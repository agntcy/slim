cask "slimctl" do
  version "3.2.0"

  if Hardware::CPU.intel?
    sha256 "78acaed6dc7da6f1523d213fed9b17a4c4c4bb96755a15532f8e13b0f7d3674f"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.2.0/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "2d056adfad1df5425acde7d2b5bf408fe124d723d3a6a0b33178040b1bd96a14"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.2.0/slimctl-darwin-arm64.tar.gz"
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
