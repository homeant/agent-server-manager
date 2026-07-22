#!/usr/bin/env bash
set -euo pipefail

readonly source_repo="homeant/agent-server-manager"
readonly tag="${ASVC_TAG:-}"
readonly tap_root="${1:-}"

if [[ -z "$tag" ]]; then
  echo "ASVC_TAG is required (for example: v0.4.1)" >&2
  exit 1
fi
if [[ ! "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+([.-][0-9A-Za-z.-]+)?$ ]]; then
  echo "invalid release tag: $tag" >&2
  exit 1
fi
if [[ -z "$tap_root" || ! -d "$tap_root/Formula" ]]; then
  echo "usage: ASVC_TAG=vX.Y.Z $0 <homebrew-tap-directory>" >&2
  exit 1
fi

readonly api_url="https://api.github.com/repos/${source_repo}/releases/tags/${tag}"
readonly formula_path="$tap_root/Formula/asvc.rb"

release_json=$(curl --fail --silent --show-error --location \
  --header "Accept: application/vnd.github+json" \
  --header "User-Agent: homeant-agent-server-manager" \
  "$api_url")

release_tag=$(jq --raw-output '.tag_name' <<<"$release_json")
checksum_url=$(jq --raw-output \
  '.assets[] | select(.name == "SHA256SUMS") | .browser_download_url' \
  <<<"$release_json")

if [[ "$release_tag" != "$tag" ]]; then
  echo "release tag mismatch: requested $tag, received $release_tag" >&2
  exit 1
fi
if [[ -z "$checksum_url" || "$checksum_url" == "null" ]]; then
  echo "release $tag does not contain SHA256SUMS" >&2
  exit 1
fi

version=${tag#v}
arm_archive="asvc-v${version}-darwin-arm64.tar.gz"
intel_archive="asvc-v${version}-darwin-x64.tar.gz"
checksums=$(curl --fail --silent --show-error --location "$checksum_url")

checksum_for() {
  local archive=$1
  awk -v archive="$archive" '$2 == archive { print $1 }' <<<"$checksums"
}

arm_sha=$(checksum_for "$arm_archive")
intel_sha=$(checksum_for "$intel_archive")
if [[ ! "$arm_sha" =~ ^[0-9a-f]{64}$ ]]; then
  echo "missing or invalid checksum for $arm_archive" >&2
  exit 1
fi
if [[ ! "$intel_sha" =~ ^[0-9a-f]{64}$ ]]; then
  echo "missing or invalid checksum for $intel_archive" >&2
  exit 1
fi

temporary_formula=$(mktemp "${formula_path}.tmp.XXXXXX")
trap 'rm -f "$temporary_formula"' EXIT
cat >"$temporary_formula" <<FORMULA
class Asvc < Formula
  desc "Shared local development service manager for humans and agents"
  homepage "https://github.com/${source_repo}"
  version "${version}"
  license "MIT"

  depends_on :macos

  on_macos do
    on_arm do
      url "https://github.com/${source_repo}/releases/download/${tag}/${arm_archive}"
      sha256 "${arm_sha}"
    end

    on_intel do
      url "https://github.com/${source_repo}/releases/download/${tag}/${intel_archive}"
      sha256 "${intel_sha}"
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
FORMULA

mv "$temporary_formula" "$formula_path"
trap - EXIT
echo "rendered asvc $version"
