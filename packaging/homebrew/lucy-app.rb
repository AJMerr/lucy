# Homebrew cask for Lucy's app, with `lucy` inside. The release workflow fills in the version and
# checksum and pushes it to AJMerr/homebrew-lucy as Casks/lucy.rb.
cask "lucy" do
  version "@VERSION@"
  sha256 "@DMG_SHA256@"

  url "https://github.com/AJMerr/lucy/releases/download/v#{version}/Lucy_#{version}_aarch64.dmg"
  name "Lucy"
  desc "Personal AI assistant with a desktop sprite, chat window and tray icon"
  homepage "https://ajmerr.github.io/lucy/"

  depends_on arch: :arm64
  depends_on macos: ">= :big_sur"

  app "Lucy.app"
  binary "#{appdir}/Lucy.app/Contents/MacOS/lucy"

  zap trash: [
    "~/Library/Application Support/lucy",
    "~/Library/LaunchAgents/io.github.ajmerr.lucy.core.plist",
  ]

  caveats <<~EOS
    Lucy isn't signed with a paid Apple certificate. If macOS won't open it, right-click Lucy in
    Applications and choose Open, or install with --no-quarantine.
  EOS
end
