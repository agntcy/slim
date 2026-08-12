cask "slimctl" do
  version "2.1.1"

  if Hardware::CPU.intel?
    sha256 "ab875778dd2fd0c369b61bc0d5653ec1545e4d730fc67de4c1d71fc55fd92827"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v2.1.1/slimctl-darwin-amd64.tar.gz"
  else
    sha256 "13797a23f45e5f1f5af5bb9af50187510eeee66b934b93ad2bd62355d67e4a20"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v2.1.1/slimctl-darwin-arm64.tar.gz"
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
