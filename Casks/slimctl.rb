cask "slimctl" do
  version "3.4.4"

  if Hardware::CPU.intel?
    sha256 "12bf908746c22c74f77a6325d522f014cd479d1cdbde142d99e01d5b81e2d462"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.4.4/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "5a8c4d7756342c47d82b9b5c9743121489b882a5cd417e2fcda9eaa5a163865d"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v3.4.4/slimctl-darwin-arm64.tar.gz"
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
