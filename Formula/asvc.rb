class Asvc < Formula
  desc "Shared local development service manager for humans and agents"
  homepage "https://github.com/homeant/agent-server-manager"
  version "0.4.0"
  license "MIT"

  depends_on :macos

  on_macos do
    on_arm do
      url "https://github.com/homeant/agent-server-manager/releases/download/v0.4.0/asvc-v0.4.0-darwin-arm64.tar.gz"
      sha256 "ff0e99e8ba33fbfaafbf64d281f5c77a73d209c7903744e39e0893dc32d5f04d"
    end

    on_intel do
      url "https://github.com/homeant/agent-server-manager/releases/download/v0.4.0/asvc-v0.4.0-darwin-x64.tar.gz"
      sha256 "6b819051198cf84c2e02d5d6b41e16ca5400f7496abe79c97ef05d6226bac2d9"
    end
  end

  def install
    bin.install "asvc"
  end

  test do
    assert_equal version.to_s, shell_output("#{bin}/asvc --version").strip
    assert_match "daemon", shell_output("#{bin}/asvc daemon status")
  end
end
