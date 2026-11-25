cask "slimctl" do
  version "3.5.1"

  if Hardware::CPU.intel?
    sha256 "8d31597d24370347ae696acc95daf47f37d64bb8cbe5a35d6ad28720a1008e9d"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.5.1/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "fa834ed77bbfd2efb3bac39d98f72468748d0d4a1154ceb25e8249c52ba4860d"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.5.1/slimctl-darwin-arm64.tar.gz"
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
