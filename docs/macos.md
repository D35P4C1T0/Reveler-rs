# macOS installation

Quit your DAW, verify the downloaded ZIP against the release's `SHA256SUMS.txt`,
and extract it. Copy the complete bundles into:

- VST3: `~/Library/Audio/Plug-Ins/VST3/`
- AU: `~/Library/Audio/Plug-Ins/Components/`
- CLAP: `~/Library/Audio/Plug-Ins/CLAP/`

The current releases are ad-hoc signed and **not Apple-notarized**. macOS may
block a downloaded bundle or offer to move it to the Trash. For a release you
have verified and trust, check its signature and remove quarantine from only
that bundle. For VST3 and AU:

```sh
codesign --verify --deep --strict "$HOME/Library/Audio/Plug-Ins/VST3/Reveler-rs.vst3"
codesign --verify --deep --strict "$HOME/Library/Audio/Plug-Ins/Components/Reveler-rs.component"
# Continue only if both checks succeed and you trust the download.
xattr -dr com.apple.quarantine "$HOME/Library/Audio/Plug-Ins/VST3/Reveler-rs.vst3"
xattr -dr com.apple.quarantine "$HOME/Library/Audio/Plug-Ins/Components/Reveler-rs.component"
```

Ad-hoc signing checks bundle integrity, not publisher identity. These commands
do not disable Gatekeeper globally. Reopen REAPER and rescan VST plugins; if it
cached a failed scan, use its VST re-scan menu to retry failed plugins. Restart
REAPER after replacing an AU bundle.

Removing the warning for all downloads requires a Developer ID Application
certificate and Apple notarization during release packaging. See
[Apple's notarization guide](https://developer.apple.com/documentation/security/notarizing-macos-software-before-distribution).
