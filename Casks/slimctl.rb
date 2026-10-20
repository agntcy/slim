cask "slimctl" do
  version "3.4.6"

  if Hardware::CPU.intel?
    sha256 "4d948671e50bea34963bb2586f05fd6a29948e79185c11ea671f6bddaaf74932"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.4.6/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "397152120d397f8b1ba229ebc20e2037f6e136307cc9725dfca83e07d7774579"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.4.6/slimctl-darwin-arm64.tar.gz"
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
