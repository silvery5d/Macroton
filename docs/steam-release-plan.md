# Steam release work plan

Status: preparation. This repository is source code; there is no verified
Steam-distributable macOS app bundle or Steamworks submission here yet.

## Current gate — 2026-10-21

The local public-source checkout now has a contributor format guide, parser validation
for compatibility and eligibility files, a pull request template, and a
mechanical source-only check. These checks run locally; the commits have not
yet reached the public remote, so remote CI has not been observed.

The SwiftUI development shell is available before a repeatable gameplay pass.
The next product gate is one eligible game with
a verified owned Steam install, visible gameplay, input, audio, save/reload,
Cloud policy, and stable exit on a supported Mac. The installed runtime and
exact release artifact still need license review, signing, notarization, and
a clean-Mac test.

An unsigned SwiftUI development preview now exercises the intended list and
Prepare/Run states; see [macOS app preview](macos-app-preview.md). This does not
move the repeatable-gameplay gate or establish a distributable release.
The preview's source-reviewed [local data and privacy draft](privacy-data-flow.md)
is ready for artifact and support-workflow review; it is not a published
customer privacy statement. The app's native-Mac eligibility filter was
verified with local installed-game inventory and automated tests. The rebuilt
unsigned app has not received a fresh window-level check because launching it
was rejected by automatic approval review pending explicit user authorization.

## 1. Product and license boundary

- Decide the first public promise: launcher for owned Steam games with reviewed
  configurations, plus clearly labeled experimental profiles. Keep game
  ownership checks and game files under Steam's control.
- Audit every redistributed artifact and its license before bundling a runtime.
  The MIT license covers Macroton-owned source, not Wine, DXVK, GPTK, Steam, or
  game payloads. Provide license notices and source obligations for any approved
  third-party components.
- Publish a privacy statement describing local game inspection, diagnostic
  files, and any network calls. Keep credentials and raw account logs out of
  issue reports and release packages.

## 2. Installable Mac product

- Create a signed 64-bit macOS app bundle with a user-facing launch path,
  first-run setup, profile selection, errors, and uninstall instructions.
- Select one reproducible runtime packaging or acquisition route and verify it
  on a clean Mac. Keep optional GPTK clearly separate from the standard path.
- Test installation, Steam authentication, launch, game exit, save/reload, Cloud
  sync, and failure recovery on supported macOS/chip combinations with games
  the testers own. Publish a conservative compatibility matrix.
- Sign and notarize the exact release artifact. Test the downloaded artifact,
  not only a local development binary.

## 3. Steamworks and store

- With a Steamworks partner account, complete required tax/banking paperwork
  and the Steam Direct app fee. Record the AppID only after Steam assigns it.
- Prepare truthful store copy, capsule art, screenshots of the actual product,
  requirements, support contact, privacy and third-party license information.
- Configure a macOS depot and launch option; upload a test build through
  SteamPipe, validate installation from a separate Steam account, then submit
  store page and build for Valve review. Do not set a release date from a code
  milestone alone.
- Release only after the review gates, Steam's timing rules, and real install
  testing have passed.

Valve's current documentation for these account and platform gates:
[Steam Direct fee](https://partner.steamgames.com/doc/gettingstarted/appfee),
[macOS platform requirements](https://partner.steamgames.com/doc/store/application/platforms),
and [SteamPipe uploading](https://partner.steamgames.com/doc/sdk/uploading).
