cask "slimctl" do
  version "1.0.0"
  
  if Hardware::CPU.intel?
    sha256 "9013a236518acf6af2c58a8d79dc62616404f826c7d8c1dadbc0b1639fcf0b2c"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v1.0.0/slimctl_1.0.0_darwin_amd64.tar.gz"
  else
    sha256 "bab8863841f90d5dad96a01e6c30ac873b4d5574847560a9a5aa0ba8ed28219a"
    url "https://github.com/agntcy/slim/releases/download/slimctl-v1.0.0/slimctl_1.0.0_darwin_arm64.tar.gz"
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
