cask "slimctl" do
  version "0.0.0-test4"
  
  if Hardware::CPU.intel?
    sha256 "4bee5f913a8c8a323d99c2f18ec99b14813ed835464f7be9c9d95c3f74f2683a"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v0.0.0-test4/slimctl_0.0.0-test4_darwin_amd64.tar.gz"
  else
    sha256 "875f4d3e996f17374fd0b974f64dc0879f7842b3ed560d54a61703ad76f7f640"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v0.0.0-test4/slimctl_0.0.0-test4_darwin_arm64.tar.gz"
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
