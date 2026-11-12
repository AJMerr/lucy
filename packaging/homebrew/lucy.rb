# Homebrew formula for `lucy` (the command and core), built from source. The release workflow
# fills in the version and checksum and pushes it to AJMerr/homebrew-lucy as Formula/lucy.rb.
class Lucy < Formula
  desc "Model-agnostic personal AI assistant: terminal chat, tools, memory, scheduled tasks"
  homepage "https://ajmerr.github.io/lucy/"
  url "https://github.com/AJMerr/lucy/archive/refs/tags/v@VERSION@.tar.gz"
  sha256 "@SOURCE_SHA256@"
  license any_of: ["MIT", "Apache-2.0"]
  head "https://github.com/AJMerr/lucy.git", branch: "main"

  depends_on "rust" => :build

  def install
    system "cargo", "install", *std_cargo_args(path: "crates/lucy-cli")
  end

  def caveats
    <<~EOS
      To keep Lucy's core running from login: lucy service install
    EOS
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/lucy --version")
  end
end
