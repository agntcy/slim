cask "slimctl" do
  version "3.4.5"

  if Hardware::CPU.intel?
    sha256 "853d45a96c08e9fcd60e4c7c5de57ff7187a935d1661bab6af782ffc9cbc54e3"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.4.5/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "0cef524f0e08fceb837c152f6fb04d55c6df26fdc4c36115b9f0a029f9f0379e"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.4.5/slimctl-darwin-arm64.tar.gz"
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
