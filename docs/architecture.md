# Architecture

## Product boundary

Macroton is a macOS game compatibility orchestrator with its own redistributable
runtime distribution. The runtime is built from WineHQ source plus a small,
reviewable Macroton patch set. Macroton's owned surface is Steam library
discovery, evidence collection, per-game policy, runtime selection, prefix
lifecycle, save mapping, and reproducible launch results.

CrossOver is not a product input: Macroton does not require, bundle, load, or
register it. A clean-machine release test must pass with only the managed
open-source runtime installed.

The current product milestone is 2D-first: promoted profiles must demonstrate
reliable 2D presentation, input, audio, save/load, Steam Cloud behavior and
sustained playability. 3D rendering and frame-rate work may remain in
experimental profiles, but is not part of the current support promise or
promotion gate.

The first supported strategy is:

```text
installed Steam game
  -> compatibility profile
  -> Windows Steam-owned payload
  -> pinned Wine WoW64 runtime
  -> graphics backend selected by policy
  -> macOS
```

The following are out of scope for the first phase:

- 32-bit Mach-O execution;
- reimplementation of Carbon, AppKit, or old macOS frameworks;
- DRM bypass or unauthenticated depot acquisition;
- kernel extensions, anti-cheat, and hardware drivers;
- silently changing the user's existing Wine or Steam installation.

## Core contracts

### Discovery

Input is a Steam root or `steamapps` directory. Output contains provenance for
each fact: the manifest path, install directory, observed binary formats, and
classification. Depot availability is not inferred from mounted local content.

### Compatibility profile

A profile is versioned, belongs to exactly one AppID, pins a runtime family and
version, selects a renderer, and names a relative executable. Unknown keys are
rejected so misspellings cannot silently change a launch.

### Launch planning

Planning is pure and read-only. It resolves paths, validates containment, and
lists blockers. Execution consumes only a blocker-free plan and emits a result
record containing the exact profile and runtime fingerprints.

For multi-depot Windows builds, the profile pins an ordered list of depot and
manifest IDs. Execution constructs a profile-addressed merged tree containing
directories and absolute symbolic links to the original owned depot files.
Differing files at the same relative path fail closed; Macroton never guesses
an overwrite order. The generated Steam manifest and Macroton result record
both retain the complete depot set.

An authenticated Windows Steam client may instead be the authority that has
already assembled and executed signed install scripts for a multi-depot tree.
`windows_payload_root=steam-installed-tree` records that provenance explicitly
and uses the committed install directory without pretending it came from a
native Steam depot export. The default `native-depot-content` mode remains the
independent-root/no-copy-merge path.

The scanner recognizes Windows drive-letter library paths emitted by Steam
inside Wine and resolves them through the containing prefix. Planning checks a
pinned BuildID and each installed depot manifest against `appmanifest_*.acf`;
identity drift is a blocker, not an implicit compatibility-profile upgrade.

Some retired games put an obsolete web launcher in Steam's public launch slot.
`steam_launcher_bypass` is a narrowly scoped payload transform for this case:
execution forces a profile-addressed private view, verifies that the declared
launcher link resolves to the expected owned payload file, and replaces only
that link with a checksum-verified copy of the profile's real executable.
Steam remains the launcher and ownership authority. Macroton refuses this
transform for direct launches, equal/escaping paths, unexpected links and
conflicting regular files; it never rewrites the Steam-owned source tree.

### Controlled execution

Execution consumes only a blocker-free plan. Arguments are passed directly to
the runtime without shell evaluation. Each AppID receives an isolated prefix,
and profiles cannot set launcher-owned variables such as `WINEPREFIX`, `HOME`,
`PATH`, or dynamic-loader injection variables. A result record includes the
runtime artifact SHA-256 and process timing. Windows Steam authentication is a
separate adapter and remains required for protected games.

Direct execution waits for the game process and records its exit code. A Steam
AppID request returns after successfully spawning the managed runtime because
Steam itself is long-lived. Its `exit_code` is deliberately `null`; downstream
compatibility evidence must come from Steam logs, the actual game process,
WindowServer presentation, and input response.

### Distribution and launch authentication are separate

The full Windows Steam client currently incurs a CEF-dominated footprint above
1.6 GiB on the test host. Macroton therefore treats payload distribution and
Steamworks launch authentication as distinct services. A bounded SteamCMD
experiment consumed roughly 84–167 MiB while updating itself, making it a
candidate downloader. It is not yet accepted for owned desktop games, and it
does not by itself provide DRM, Cloud, achievements, or an authenticated
Steamworks session. See `docs/evidence/steamcmd-memory-2026-08-13.md`.

### Windows Steam bootstrap

Bootstrap accepts an explicit installer path and expected SHA-256. It verifies
the installer is a Windows PE executable, resolves the exact runtime pinned by
the compatibility profile, and silently installs into `games/<appid>/prefix`.
The committed record contains both artifact fingerprints. Existing prefixes
without a matching record are never adopted, and authentication data is never
copied from native Steam, CrossOver, or another Wine prefix.

### Runtime acquisition

Runtime installation is explicit and local in v0.1. Its contract requires:

- an explicit artifact source and license metadata;
- a cryptographic checksum;
- atomic installation;
- no modification of system Wine or the user's global prefix;
- a stable runtime ID referenced by result records.

Automatic network acquisition remains absent. It will require a signed catalog
policy and a reviewed redistribution decision for each runtime family.

### Runtime independence

The supported baseline is `Macroton Runtime`, produced reproducibly from public
source and shipped with Macroton. Its initial component policy is:

- WineHQ WoW64 as the Windows execution engine;
- WineD3D/OpenGL for the simplest legacy OpenGL and Direct3D cases;
- DXVK plus MoltenVK where their feature intersection works on macOS;
- no dependency on CrossOver, D3DMetal, or another commercial runtime.

For Steam launches, Macroton disables injection of the optional 32-bit and
64-bit Steam overlay renderer DLLs. Silver demonstrated that the overlay can
block indefinitely during audio initialization before the game creates a
window. This policy does not replace `steam_api.dll`, suppress ownership
validation, or bypass authentication; it only removes the optional in-game
overlay from managed child processes.

The Steam launch resource guard also bounds the handoff: after observing a
Steam client but no configured Windows game for five minutes, it records a
`steam-client-no-game` resource evidence event and stops only that AppID's
prefix. This prevents a failed `steam_api.dll`/Steam IPC handoff from leaving
CEF processes resident indefinitely or being reported as playable.

Steam console events that wait for `SynchronizingCloud "pendingcloudsessions"`
are exposed as the distinct `waiting-cloud` lifecycle state, rather than the
generic `waiting-user` state, so a launcher can direct the user to resolve
Cloud synchronization before retrying the game.

## Why the GUI comes later

The SwiftUI launcher should render core state, not invent it. Building the
scanner and plan contracts first keeps the GUI honest and makes behavior
testable with fixtures before any runtime download or game launch is attempted.
