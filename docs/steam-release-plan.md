# Steam release work plan

Status: preparation. This repository is source code; there is no verified
Steam-distributable macOS app bundle or Steamworks submission here yet.

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
