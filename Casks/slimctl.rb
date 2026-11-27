cask "slimctl" do
  version "3.5.2"

  if Hardware::CPU.intel?
    sha256 "5c1b696850a3fad076b7e0e261c55fef1ec149b945ce53af28075365685dabc0"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.5.2/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "45539bc1f087d9a1364672c5e94b22314a63caf0ca6b376574b487bdb5ad09f6"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.5.2/slimctl-darwin-arm64.tar.gz"
  end

  name "slimctl"
  desc "A CLI tool for managing SLIM Devices"
  homepage "https://github.com/agntcy/slim"

  binary "slimctl"

  postflight_steps do
    set_permissions "slimctl", "+x", recursive: false
    run "/usr/bin/xattr", args: ["-dr", "com.apple.quarantine", "{{staged_path}}/slimctl"]
  end
end
