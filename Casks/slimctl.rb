cask "slimctl" do
  version "2.0.0-alpha.2"

  if Hardware::CPU.intel?
    sha256 "0c3edebaccf70017776553b86159d0703eea823a022ae6db6958f754fec0fabc"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0-alpha.2/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "2d0a43b3a49005b2458fe93cd1fdd4ddb5b0ffb5334a323829c4f230fd4ddaa5"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v2.0.0-alpha.2/slimctl-darwin-arm64.tar.gz"
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
