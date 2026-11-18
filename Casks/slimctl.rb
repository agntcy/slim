cask "slimctl" do
  version "3.5.0"

  if Hardware::CPU.intel?
    sha256 "b7bbdd7013ef5184efddef2199705e72046891e06117ff87bcd53e6e7c2741cc"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.5.0/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "f5e467bff9d693d8786b5d49e0519754ed4887aed1cd1b23e47939e3d98cca07"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.5.0/slimctl-darwin-arm64.tar.gz"
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
