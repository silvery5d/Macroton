# macOS app preview

Build an unsigned development preview on a Mac with Rust, Swift, Xcode command
line tools, and MinGW-w64:

```sh
sh scripts/build-macos-app.sh
open dist/Macroton.app
```

The app reads **locally installed** games from the native Steam library and
Macroton-managed Windows Steam libraries, combining duplicate AppIDs. It
uses a three-pane native macOS window: a translucent category sidebar, a
searchable local library, and selected-game details with the Prepare/Run action
and command status. The dark gray palette and restrained borders follow the
[StyleKit macOS Vibrancy reference](https://www.stylekit.top/zh/styles/macos-vibrancy).
The sidebar uses `NSVisualEffectView`; the other panels use solid dark grays.
It
does not read Steam credentials or claim to list every purchased game. The
`Run` button appears only when a matching configuration, Windows Steam install,
Windows game manifest, executable, and runtime pass the local launch plan.
For a game installed in another Macroton-managed Windows Steam library, the
app also checks that the resident Steam client is live, its profile and runtime
provenance match, and it is the only eligible shared client. The shared-client
button state can change when that client exits or another one starts.
This is readiness to attempt a launch, not a gameplay certification.

`Prepare` imports a locally supplied `APPID.conf` if no configuration is
available, then advances the existing Macroton/Steam flow. The first Windows
Steam setup requires a Valve installer and an independently verified SHA-256.
The account owner handles Steam sign-in, purchase or EULA prompts, and install
confirmation in Steam. When signed in, `Prepare` requests the Windows version
from Steam and subsequent refreshes show its download state. Macroton does not
bundle or mirror Steam/game payloads. A missing compatible runtime is reported
as a prerequisite; runtime acquisition is not automated in this preview.

The preview copies bundled configurations into
`~/Library/Application Support/Macroton/profiles` on first use. It preserves
existing files in that directory. New contributed configurations must be
imported explicitly; a remote configuration catalog is not connected yet.

The current package is unsigned and unnotarized. It has not passed clean-Mac,
Steam ownership, Windows download, gameplay, or SteamPipe installation tests.
