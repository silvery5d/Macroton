# Macroton development preview: local data and privacy

Status: source-reviewed draft for the unsigned development preview. Recheck
against the exact signed release artifact, bundled runtime, support workflow,
and Steamworks configuration before publishing a customer privacy statement.

## What the preview reads

- The native Steam library's local manifests and installed game binaries to
  build the game list and identify native macOS executables. An installed
  manifest is not proof that an account owns a game.
- Macroton-managed Windows Steam library manifests, installed game paths,
  Steam client state, and local logs to determine whether Prepare or Run can
  be attempted. These checks can encounter account names, tickets, save paths,
  and other private material in files maintained by Steam.
- Compatibility profiles and eligibility reviews bundled with the app, plus
  profiles imported by the user. The profile validator checks a selected
  contributor file before copying it into Macroton's profile directory.

## What the preview keeps locally

- Imported and first-run bundled profiles are kept in
  `~/Library/Application Support/Macroton/profiles`. Existing profiles are not
  overwritten by first-run setup.
- Managed Windows Steam prefixes, installation and launch state, runtime
  registration, and diagnostic artifacts can be kept under
  `~/Library/Application Support/Macroton`. A managed Steam client may keep its
  own login state, game files, saves, and logs inside that directory.
- The app shows the last command output in its window. Some command output or
  Steam logs may include local paths or account-related details. Review and
  redact diagnostics before sharing them in an issue or support request.

## Network and sharing boundary

The preview has no Macroton account, telemetry endpoint, or automatic
diagnostic upload in the reviewed source. Macroton can start Valve's Windows
Steam client or SteamCMD, which communicate with Steam for sign-in, ownership,
downloads, updates, and Cloud features under [Valve's privacy policy](https://store.steampowered.com/privacy_agreement/).
No Steam credentials, tickets, game payloads, raw logs, or saves should be
included in public configuration contributions. The preview does not infer
ownership from local installation or cached metadata.

## Removing local data

Removing the app bundle alone does not remove Macroton's Application Support
directory. Before removing that directory, check whether the managed Windows
Steam client contains saves or other data you want to retain, and resolve any
pending Steam Cloud handoff. Macroton does not delete the native Steam library
when its own app bundle is removed.

## Release review still required

- Audit network behavior and retained data from the exact bundled runtime and
  signed build, including Steam and any third-party components.
- Verify the final support contact, diagnostics submission process, retention
  and deletion instructions, and any jurisdiction-specific disclosures.
- Publish a customer-facing statement only after those facts are confirmed.
