> This source snapshot omits runtime binaries, Steam/game payloads, local profiles,
> and raw validation evidence. Historical evidence references in the documentation
> describe private test records and are not part of this source release.

# Macroton

To submit a configuration for a game, see [CONTRIBUTING.md](CONTRIBUTING.md).
The [Steam release work plan](docs/steam-release-plan.md) tracks the remaining
product, packaging, and Steamworks gates.

Macroton is an evidence-first compatibility launcher for legacy Steam games on
modern macOS. Its first execution path uses a Windows build of a game through a
Wine WoW64 runtime. It does **not** emulate 32-bit macOS or bypass Steam DRM.
Games with an official macOS build that already runs on the target Mac are not
Macroton compatibility targets; native viability is checked before a Windows
depot is selected.

## Current product scope

Macroton prioritizes **legacy 2D Win32 Steam games** whose macOS builds are no
longer usable on modern 64-bit macOS. Installed PE32+ x86-64 Windows games may
also be validated in a separately labelled experimental Win64 2D lane; their
results never count as Win32 coverage. An installed game entrypoint, rather
than a bundled helper or a store architecture label, determines the lane.
2D means the acceptance path is
focused on menus, sprites/UI, input, audio, save/load, Steam Cloud and sustained
playability; 3D scene rendering, DXVK/Metal tuning and 3D frame-rate claims are
out of scope for the current product milestone. 3D experiments remain useful
diagnostics, but must not be promoted as supported compatibility results.

The repository currently contains the first vertical slice:

- discover every local Steam library;
- parse installed app manifests;
- inspect installed Mach-O and PE payloads;
- distinguish local evidence from unverified depot availability;
- load a per-AppID compatibility profile;
- produce a dry-run launch plan without mutating a Wine prefix.
- verify and atomically install a pinned runtime artifact.
- bootstrap Windows Steam into an audited, isolated per-AppID prefix.
- execute a blocker-free plan in an isolated per-AppID prefix and record the result.
- suppress Steam's optional overlay DLL injection in managed Steam launches
  while preserving ownership checks and Steamworks.
- preserve required game arguments when reusing a live Steam client; protocol
  retries are limited to argument-free launches so launcher-bypass profiles do
  not lose their handoff contract.
- assemble ordered multi-depot games as a no-copy, conflict-detecting payload
  view and record every depot manifest in Steam and Macroton evidence.

## Build and run

Use `macroton play APPID` for the configured Steam login, install, and launch
flow. Pending Steam confirmation does not establish whether an account owns a
game. `macroton shortcut APPID` generates a local Finder `.command` file named
from the portable `shortcuts/catalog.tsv` game list. See
`shortcuts/README.md` to contribute a game name and regenerate shortcuts on
another machine.

When switching games, `play` waits for the previous game's own Steam client
to observe game exit and a post-exit Cloud sync whose declared local files
match its cache. It then stops only that idle AppID client while preserving its
login and starts the new game. A pending, bypassed, failed, or unverified upload
blocks the switch; Macroton never selects between conflicting save versions.
If the previous Steam client died before completing sync, the pending handoff
marker keeps the next game from silently replacing that session. This may add
waiting time before the next game starts. A shared save directory alone cannot
make a different AppID's Steam client upload the first game's files; Steam
Cloud sync is tied to each game's launch and exit
([Steamworks Cloud documentation](https://partner.steamgames.com/doc/features/cloud)).

The shared-client migration starts with a read-only check:

```sh
macroton steam shared-client-status 400 26800
```

It compares the two profiles' runtime and Steam child-process settings,
checks the resident client's exact prefix/session provenance, and checks
whether the target is installed in that client's authenticated Steam library.
An installed state here is not an ownership assertion for an unconfirmed
account. A `candidate` result is a prerequisite for shared launch, not proof
that Steam has created the target game process or a window.
For a compatible live client, `macroton steam shared-client-install A B`
requests B's installation inside A's authenticated Steam library after A's
post-exit Cloud gate. Steam retains purchase and installation confirmation.
The command does not start B or move an existing B installation or save.
When a compatible resident client already has B installed, `macroton play B`
submits B's normal Steam launch to that client; otherwise it asks that same
client to install B. `macroton steam shared-client-launch A B` selects A
explicitly. Shared launch currently requires B to have no game-specific Steam
arguments or launcher bypass. B's launch state and Cloud files are read from
A's prefix. Before another game starts, Macroton waits for B to exit and for
its required Cloud upload. `macroton steam shared-client-finish A B` performs
that check and releases B's pending marker. If no compatible client exists,
`play` uses the guarded sequential handoff above. A shared launch request is
not gameplay or remote freshness proof; check its status and Steam's Cloud UI.
If Steam never created B's game process, `macroton steam shared-client-abandon A B`
first checks both game logs and Cloud operations, stops A's client while
preserving login, then clears the abandoned marker. It refuses a game or
unresolved Cloud prompt.

```sh
cargo test
scripts/test-macos-swift-tools.sh
cargo run -- scan
cargo run -- scan --json
cargo run -- inspect "/path/to/Game.app"
cargo run -- plan 123456 --profile compatibility/123456.conf
cargo run -- launch 123456 --profile compatibility/123456.conf
cargo run -- runtime verify runtimes/runtime.conf /path/to/runtime.tar.xz
cargo run -- runtime install runtimes/runtime.conf /path/to/runtime.tar.xz
cargo run -- runtime list
cargo run -- runtime discover
cargo run -- runtime audit /absolute/path/to/wine
cargo run -- runtime release-audit \
  runtimes/macroton-winehq-macos-11.14.conf --runtime-root /path/to/runtimes
cargo run -- runtime probe-win32 /absolute/path/to/wine
cargo run -- steam bootstrap 123456 --profile compatibility/123456.conf \
  --installer /path/to/SteamSetup.exe --installer-sha256 EXPECTED_SHA256
cargo run -- steam prefix-status 123456
cargo run -- steam prefix-status 123456 --json
cargo run -- steam inventory --steam-root /path/to/Steam \
  --eligibility-dir eligibility --with-appinfo --candidates-only --json
cargo run -- steam inventory --steam-root /path/to/windows/Steam \
  --eligibility-dir eligibility --verified-win32-only --json
cargo run -- steam inventory --steam-root /path/to/windows/Steam \
  --eligibility-dir eligibility --2d-only --json
cargo run -- steam inventory --steam-root /path/to/windows/Steam \
  --data-root /path/to/macroton-data --eligibility-dir eligibility \
  --verified-win32-only --2d-only --json
cargo run -- steam cloud-status APPID --profile compatibility/APPID.conf \
  --data-root /path/to/macroton-data --json
cargo run -- steam cloud-wait APPID --profile compatibility/APPID.conf \
  --data-root /path/to/macroton-data
cargo run -- steam connection-status 123456
cargo run -- steam appinfo 123456
cargo run -- steam library-status 123456
cargo run -- steam inventory --steam-root /path/to/windows/Steam \
  --min-playtime 60 --limit 50 --with-appinfo
cargo run -- steam inventory --steam-root /path/to/windows/Steam \
  --min-playtime 60 --limit 50 --candidates-only
cargo run -- steam target-status APPID \
  --native-steam-root /path/to/native/Steam \
  --windows-steam-root /path/to/windows/Steam \
  --eligibility-review eligibility/APPID.conf --json
cargo run -- evidence snapshot 606680 --profile compatibility/606680.conf \
  --data-root /path/to/macroton-data --runtime-root /path/to/macroton-runtimes \
  --output /absolute/path/snapshot.json
cargo run -- evidence verify-audio 606680 --profile compatibility/606680.conf \
  --data-root /path/to/macroton-data --probe /absolute/path/audio-probe.json \
  --runtime-root /path/to/macroton-runtimes \
  --output /absolute/path/audio-evidence.json
cargo run -- evidence verify-focus-stream 606680 \
  --profile compatibility/606680.conf \
  --data-root /path/to/macroton-data --runtime-root /path/to/macroton-runtimes \
  --probe /absolute/path/focus-stream.csv \
  --output /absolute/path/focus-stream-evidence.json
cargo run -- evidence verify-interaction 606680 \
  --profile compatibility/606680.conf \
  --data-root /path/to/macroton-data --runtime-root /path/to/macroton-runtimes \
  --probe /absolute/path/interaction.json \
  --capture-dir /absolute/path/interaction-capture \
  --output /absolute/path/interaction-evidence.json
cargo run -- evidence verify-independence 606680 \
  --profile compatibility/606680.conf \
  --data-root /path/to/macroton-data --runtime-root /path/to/macroton-runtimes \
  --output /absolute/path/runtime-independence-evidence.json
cargo run -- evidence watch-exit 606680 --profile compatibility/606680.conf \
  --data-root /path/to/macroton-data --runtime-root /path/to/macroton-runtimes \
  --min-runtime-seconds 1800 --timeout-seconds 14400 \
  --output /absolute/path/normal-exit-evidence.json
```

After the individual live gates and fresh Cloud transaction, build one
create-new automated acceptance bundle:

```bash
macroton evidence bundle APPID --profile PATH \
  --focus /absolute/focus.json --motion /absolute/motion.json \
  --interaction /absolute/interaction.json --audio /absolute/audio.json \
  --independence /absolute/independence.json --exit /absolute/exit.json \
  --data-root /path/to/data --runtime-root /path/to/runtimes \
  --output /absolute/automated-bundle.json
```

The bundle rejects evidence assembled across different AppIDs, profiles,
runtime fingerprints, Steam sessions, game PIDs, client modes, renderers or
graphics APIs. It also requires a completed fresh Cloud roundtrip, current
Steam exit code 0 and zero exact-prefix residual processes/RSS. A recovered
`restored` Cloud transaction is not a roundtrip pass. Source files are hashed
into the bundle. This is the automated half of acceptance only: correct color,
audible quality, semantic input/gameplay, in-game save reload and human
playability remain explicit review gates.

Before or during a run, place the six canonical files at
`focus.json`, `motion.json`, `interaction.json`, `audio.json`,
`independence.json` and `exit.json` in one directory, then inspect every gap
without mutating the prefix:

```bash
macroton evidence bundle-status APPID --profile PATH \
  --evidence-dir /absolute/evidence-dir \
  --data-root /path/to/data --runtime-root /path/to/runtimes

macroton evidence matrix-status \
  --profiles-dir /absolute/path/to/compatibility \
  --evidence-root /absolute/path/to/live-evidence \
  --failure-dir /absolute/path/to/failure-records \
  --failure-source APPID=/absolute/path/to/source-session-notes \
  --data-root /path/to/data \
  --app-data-root APPID=/path/to/isolated-app-data \
  --runtime-root /path/to/runtimes
```

The JSON reports each artifact as `missing`, `invalid` or `valid`, then reports
cross-artifact consistency, Cloud phase, exit-code gate, zero-residual gate and
the final `automated_bundle_ready` boolean. Schema 2 distinguishes an unresolved
runtime, missing prefix, prefix/runtime provenance mismatch and a ready runtime;
`blocking_gate` plus `next_action` identify the first ordered recovery step.
Historical exit state alone cannot make the result ready.
When `--failure-dir` is supplied, every matching human failure is re-bound to
the current profile and re-hashed against its explicit `--failure-source`.
The matrix then reports `known-human-failure` instead of losing an observed
failure among pending captures. Missing or stale source bindings fail closed.
When any live artifact is missing or invalid, the action is deliberately
`capture-one-live-session` or `recapture-one-live-session`, not an instruction
to patch one JSON file across sessions. `blocking_artifact` and its state retain
the precise diagnostic.

After an automated bundle passes, record the human-observed gates explicitly:

```bash
macroton evidence record-review APPID --profile PATH \
  --bundle /absolute/automated-bundle.json \
  --visual pass --audio pass --input pass --gameplay pass \
  --save-reload pass --sustained-playability pass \
  --output /absolute/human-review.json

macroton evidence acceptance-status APPID --profile PATH \
  --bundle /absolute/automated-bundle.json \
  --review /absolute/human-review.json
```

The review is content-addressed to the exact automated bundle and profile. A
failed visual, listening, semantic-input, gameplay, in-game save/reload, or
sustained-playability gate remains a valid failure record but cannot produce
`gameplay_acceptance_ready: true`. This prevents a launch, menu, or synthetic
probe from being promoted into a full compatibility claim.

A game may fail before it can produce a complete automated bundle. Preserve
that result instead of dropping it from the matrix:

```bash
macroton evidence record-failure APPID --profile PATH \
  --source /absolute/session-notes.md \
  --gate sustained-playability --code unacceptable-3d-frame-rate \
  --output /absolute/playability-failure.json

macroton evidence failure-status APPID --profile PATH \
  --failure /absolute/playability-failure.json \
  --source /absolute/session-notes.md
```

Failure records are profile-bound and content-addressed to their source notes.
They explicitly remain human observations rather than invented automated FPS
or session telemetry. Changing the source after recording invalidates the
record.

To collect the six live automated artifacts from one already-running,
Steam-tracked PID, use the fail-closed interactive wrapper. It compiles the
three ScreenCaptureKit probes, verifies every artifact before the session can
drift, watches a normal in-game exit, and performs an exact-prefix stop plus
zero-process/zero-RSS verification on success, failure or interruption:

```sh
scripts/capture-runtime-evidence-session.sh APPID compatibility/APPID.conf \
  auto auto /absolute/new/evidence-dir /path/to/data /path/to/runtimes \
  key escape
```

The second `auto` resolves the PID from Macroton's current Steam lifecycle.
Either auto value may be replaced explicitly for diagnosis. An explicit
WindowServer ID is required when the tracked PID owns multiple eligible
game-sized windows; automatic selection fails closed rather than guessing
between them. The final action may instead be `click X_FRACTION Y_FRACTION`.
The wrapper attaches to an existing game and deliberately does not launch
another copy. Its result still cannot replace a required fresh Steam Cloud
roundtrip or the explicit human visual, listening, semantic-input, save-reload
and stability gates. A source-bound `steam_cloud=unsupported` profile records
the network roundtrip as not applicable rather than instructing an impossible
Cloud test.

Evidence snapshots bind the selected profile to the installed runtime and the
exact AppID prefix bootstrap. Treat only `profile_runtime_match: true` as a
runtime-provenance pass; unresolved or mismatched runtimes remain `null`/`false`.
`profile_session_match` is stricter: it becomes true only when the live managed
Steam session was started with that exact profile, client mode, renderer and
runtime identity. A matching active marker is also content-addressed as
`steam_session_sha256`; idle/stopped prefixes record `null`.

`verify-focus-stream` binds the high-rate ScreenCaptureKit CSV to the same live
Steam game PID and session identity. It requires ordered focused frames,
unchanged window/PID ownership, a minimum duration/frame count, bounded delivery
gaps, and an explicit whole-frame black-luma allowance. It proves only those
temporal sampling bounds: correct color, localized flicker, input response,
scene correctness and playable frame rate remain separate gates.

For a known continuously moving 3D scene, the same raw stream can feed the
separate cadence proxy:

```bash
macroton evidence verify-motion-rate APPID --profile PATH \
  --probe /absolute/path/focus-stream.csv \
  --min-duration-seconds 10 --min-changed-rate 20 \
  --max-unchanged-ms 200 --output /absolute/path/motion-rate.json
```

This records delivered callbacks, sampled-frame changes per second, longest
unchanged interval and p95 change interval while binding the exact live game
PID/profile/runtime/session. It is useful for distinguishing the reported
sub-1-FPS behavior from a smoothly changing scene, but is explicitly a motion
presentation proxy: engine FPS, simulation rate, input latency and human
playability remain unproven. Use it only on a reviewed scene expected to move.

MoltenVK GPU captures have a separate live-provenance gate:

```bash
macroton evidence verify-gpu-capture APPID --profile PATH \
  --capture /absolute/path/frame.gputrace \
  --data-root /path/to/macroton-data \
  --runtime-root /path/to/macroton-runtimes \
  --output /absolute/path/gpu-capture-evidence.json
```

The verifier requires the exact live game PID/profile/runtime/Steam session,
rejects symbolic links, empty or changing capture trees, and records a stable
content-addressed manifest. This proves that the selected live runtime emitted
GPU command bytes; visible presentation, correct rendering, frame rate, input
and playability remain independent gates.

`tools/macos_interaction_probe.swift` performs one bounded key or window-relative
click between exact-window before/after captures. `verify-interaction` binds its
PID to the live Steam game, re-hashes both PNGs, requires uninterrupted focus
and ownership, distinct frame hashes, and an explicit minimum changed-pixel
ratio. The integrated order is stronger than separately collected input and
screenshots, but animated scenes can still change without handling the event;
semantic response and causality require human review of the captured images.

`verify-independence` inspects the live Steam-tracked game PID's open/mapped
paths. It requires positive mappings from both the selected Macroton runtime
directory and exact AppID prefix, and rejects any CrossOver or CodeWeavers path.
The persisted record includes only category counts and a SHA-256 of the sorted
path set, not local paths. This proves a bounded live-process snapshot; it does
not claim that earlier, future, or unobserved child processes used the same set.

Start `watch-exit` only after the real Steam game PID is running. It continuously
samples exact-prefix ownership/RSS, requires that tracked PID to remain observed
for the requested lower-bound interval, then accepts only Steam's removal event
with exit code zero. Its record includes peak RSS and any remaining Steam/helper
RSS, but does not claim the whole prefix is stopped; run `macroton stop APPID`
and `resource status` afterward for the separate zero-residual gate.

`scan` defaults to `~/Library/Application Support/Steam`. Use
`--steam-root PATH` for fixtures or nonstandard installations.

## Evidence states

The scanner deliberately avoids claiming that a game has no Windows build when
only its currently mounted macOS depot was inspected.

| State | Meaning |
| --- | --- |
| `native-mac` | A local arm64 or x86_64 Mach-O payload was observed. |
| `legacy-mac-32` | A local i386 Mach-O payload was observed and no modern Mac slice was found. |
| `windows-payload` | A local PE executable was observed. |
| `unknown` | The installed files do not provide enough evidence. |

Remote Steam depot discovery and authenticated download remain separate
adapters. They must preserve Steam ownership and DRM checks. Macroton can
install Valve's Windows client into an isolated prefix, but never copies login
state from another Steam installation.

`steam appinfo` reads the selected Windows client's local binary
`appcache/appinfo.vdf` and reports cached launch, BuildID, public manifest, size,
language, DLC metadata, and any declared UFS save/root-override rules. It never treats that cache as current entitlement
or installed-payload proof; the signed install manifest and real files remain
separate gates. Passing `--profile PATH` also verifies that the cached public
BuildID, install directory, launch path, and every profile-declared depot
manifest still match; extra cached language or DLC depots do not cause a false
failure. A UFS rule is authoritative path-pattern metadata, not evidence that a
matching file exists or has synchronized; conversely, the absence of UFS rules
does not prove that the title lacks API-backed Steam Remote Storage.

`steam library-status` privately summarizes matching AppID records across the
selected Windows client's local account libraries. It prints only record count,
maximum historical playtime, latest-played timestamp, and cached-ticket count;
account directory names and ticket contents never leave the parser. Historical
library or ticket cache presence is candidate evidence, not current entitlement.

`steam inventory` applies the same redaction to the whole selected local Steam
library and ranks AppIDs by maximum observed playtime and recency. It is a
candidate-discovery tool only: every result must still pass the official native
macOS viability gate, current entitlement, installed-payload, and real-game
entrypoint checks. `--with-appinfo` adds the cached application name and launch
platforms so Mac-capable candidates can be rejected early; cache age means it
does not replace the current official-platform check.

## Compatibility profiles

Profiles use a deliberately small, auditable `key=value` format:

```text
schema=1
appid=123456
runtime=wine-wow64
runtime_version=10.0
renderer=auto
executable=Game/game.exe
arguments=-windowed
env.WINEDLLOVERRIDES=xinput1_3=n,b
cloud_files=slot.sav|AppData/Roaming/Example/slot.sav
```

An unqualified Cloud local path remains relative to the Wine user directory.
Use `user:` to make that root explicit, `steam-install:` for titles whose
Cloud files live below the installed game directory (which also requires
`steam_install_dir`), or `steam-remote:` for games that access files through
Steam Remote Storage rather than a second filesystem copy:

```text
steam_install_dir=ExampleGame
cloud_files=Profiles/Player.nfp|steam-install:Profiles/Player.nfp;savegame.aws|steam-remote:savegame.aws
```

For a Windows build composed from several depots, use an ordered manifest-pinned
list instead of the legacy single-depot fields:

```text
windows_depots=402:7655374749121348667,409:5466869569815608374
```

When those depots have already been assembled by the authenticated Windows
Steam client, a profile may preserve that signed installation result with:

```text
windows_payload_root=steam-installed-tree
```

The default remains `native-depot-content`, which expects separate owned depot
roots and builds the no-copy merged view. The installed-tree mode never claims
that depot files were independently reproduced; it records the full manifest
set while using Steam's committed install directory as the payload authority.
For pinned profiles, planning compares the installed BuildID and every recorded
depot manifest with the profile and fails closed on drift. Windows drive-letter
library paths written by Steam inside Wine are mapped back through that prefix;
unavailable secondary drives are skipped rather than interpreted as macOS paths.

Macroton recursively merges those roots into a profile-fingerprinted view made
of symbolic links. It does not duplicate the payload. Directory overlap is
allowed, byte-identical duplicate files are allowed, and any differing file at
the same relative path blocks the launch rather than applying an implicit
last-writer-wins rule.

For a Steam game whose retired public launcher fails before its real game
entrypoint, a profile may declare a staged bypass:

```text
launch=steam
steam_install_dir=Age2HD
executable=aok hd.exe
steam_launcher_bypass=Launcher.exe
arguments=NoStartUp
```

Macroton then forces a private fingerprinted payload view and replaces only
that view's `Launcher.exe` with a verified copy of the configured game
executable. Steam still creates the AppID launch context and receives the
arguments. The Steam-owned install is never modified. The source launcher must
exist at exactly the declared relative path; an escaping path, a direct launch,
an unexpected link target, or any conflicting staged file blocks execution.

Do not use a direct-EXE diagnostic profile for Steam-protected games. A game
started as `wine game.exe` has no Steam-owned AppID launch context and commonly
shows a `Steam is required` dialog; that dialog is expected and does not prove
that Macroton's supported path is broken. Use `launch=steam` and let Macroton
submit `Steam.exe -applaunch APPID` inside the exact AppID prefix.

Macroton also refuses a second Steam launch while that exact prefix already has
an identified `windows-game` process. Reuse the existing game session, or run
`macroton stop APPID` against that AppID before launching again. This avoids
Steam's ambiguous `Game already running` state and keeps all evidence attached
to one game lifecycle.

### Wine Steam black-window repair

On Wine 11 under Apple Silicon, Steam's multi-process CEF helper can create a
fully black native window even while the Steam process and network session are
alive. Macroton ships a small MIT-licensed PE32+ wrapper that preserves Valve's
original helper and launches it with `--disable-gpu --single-process`:

```sh
scripts/build-steamwebhelper-wrapper.sh
scripts/install-steamwebhelper-wrapper.sh APPID
```

The installer keeps the original helper as `steamwebhelper_real.exe` and writes
`BootStrapperInhibitAll=enable` only inside that AppID's private Steam prefix.
It never modifies native Steam, another AppID, or a CrossOver bottle. Verify
the result with `file` on both helper binaries and a screenshot of a readable
Steam window before using the prefix for installation or gameplay evidence.
The manual installer requires a built Macroton binary (or `MACROTON_BIN`) to
verify that the exact AppID prefix has zero live processes; if that check fails,
it refuses to change the helper. Stop a live prefix with `--preserve-session`
before installing the wrapper.
Once installed, Macroton checks the wrapper before each managed Steam login,
install handoff, and game launch. If a Steam update replaced it, Macroton first
requires that exact prefix to be idle, preserves the previous Valve helper,
promotes the updated Valve helper to `steamwebhelper_real.exe`, and restores the
wrapper. A live prefix is never patched in place; stop it with
`macroton stop APPID --preserve-session` and retry. This prevents a successful
one-time black-window repair from silently disappearing after an update, but
still requires a visible-window check: CEF process liveness is not pixel proof.
Before every Steam-mode launch, Macroton performs a non-destructive bind check
for Steam's local TCP port `27036`. If the port is occupied (for example by a
running native macOS Steam client), it prints an advisory warning and continues;
Macroton never terminates or reconfigures that other client. If the Wine client
then remains black or cannot connect, close the conflicting native Steam session
and retry the exact AppID launch.
The resource guard classifies both `steamwebhelper.exe` and the preserved
`steamwebhelper_real.exe` as Steam CEF, preventing an experimental wrapper
from being mistaken for a game process. The wrapper is reversible and is not
enabled automatically by the standard runtime.

Paths must be relative and cannot escape the game directory. A launch plan is
only executable when the configured payload and a managed runtime installation
both exist; version 0.1 reports those blockers instead of downloading or
launching implicitly.

## Runtime installation

Runtime acquisition and installation are separate operations. Macroton does not
download an artifact implicitly. The user or a future authenticated catalog
adapter supplies an archive, then the core:

1. parses its versioned manifest;
2. checks source and license metadata;
3. verifies the full SHA-256 digest;
4. rejects unsafe archive entries;
5. extracts into a private staging directory;
6. atomically renames it into the managed runtime root.

Existing runtimes are never overwritten. The format and security contract are
documented in [runtimes/README.md](runtimes/README.md).

An already installed Wine runtime can instead be registered by absolute path.
Macroton does not copy it, and verifies its executable fingerprint before
every plan or launch.

Macroton does not require, bundle, or load CrossOver. Supported releases use a
managed open-source Wine runtime and must pass clean-machine testing with
CrossOver absent. The runtime policy is documented in
[docs/architecture.md](docs/architecture.md).

## Controlled execution

`plan` remains read-only. `launch` only accepts a blocker-free plan and:

- invokes the managed runtime directly without a command shell;
- creates a dedicated prefix at `games/<appid>/prefix`;
- prevents profiles from overriding launcher-owned environment variables;
- writes an atomic JSON result containing the runtime artifact fingerprint,
  timestamps, and process exit code.

Direct launches wait for the game and record its exit code. Steam launches
submit the AppID request and return immediately because Windows Steam is a
long-lived client; their result records intentionally contain `exit_code: null`.
That record proves only that Macroton created the request. Compatibility still
requires separate process, log, window, and input evidence.

If Steam holds a game at a Cloud launch confirmation, `cloud-status` remains
`launch_ready=false` even when file bytes match. Inspect the isolated Windows
Steam UI without changing local or remote saves with:

```sh
node scripts/resolve-steam-cloud-dialog.js \
  --webhelper-log "$MACROTON_DATA/games/APPID/prefix/drive_c/Program Files (x86)/Steam/logs/webhelper.txt"
```

The helper discovers the current localhost CEF port from `webhelper.txt` and
is read-only unless `--apply` is explicitly authorized.
`macroton launch` reports the pending operation but still submits the normal
Steam launch request, allowing Steam to show any owner-required Cloud choice.
It never chooses between local and remote saves or starts the game EXE directly.
The launch request alone does not prove that the Cloud choice was resolved or
that the game started.

Stop one managed game prefix without touching any other Wine session with:

```sh
macroton stop APPID
```

An explicit `stop` also cancels a concurrent `macroton launch` for the same
AppID while that launch is waiting for Steam client warmup or a protocol
retry. The launch command reports cancellation instead of recreating the
client after the stop. Wait for the launch command to terminate and confirm
`macroton resource status APPID` reports zero processes before treating a
diagnostic prefix as fully stopped.

For iterative owner-assisted Steam testing, `macroton stop APPID
--preserve-session` stops only that AppID's prefix processes while retaining
the Macroton Steam-session provenance marker. This does not guarantee that
Valve's online token remains valid, but avoids an unnecessary local session
reset between 2D validation attempts.

Inspect or bound the memory used by one exact managed Wine prefix without
counting native Steam or unrelated Wine sessions:

```bash
macroton resource status APPID --max-rss-mib 1600
macroton resource status APPID --max-rss-mib 1600 --json
macroton resource status-all --data-root /path/to/macroton-data \
  --max-total-rss-mib 4096 --json
macroton resource guard APPID --max-rss-mib 1600
macroton resource guard APPID --max-rss-mib 1600 --hard-rss-mib 1900
macroton resource compare APPID \
  --standard /path/to/standard.json \
  --low-memory /path/to/low-memory.json \
  --standard-profile /path/to/standard.conf \
  --low-memory-profile /path/to/low-memory.conf \
  --phase game --output /path/to/comparison.json
```

`resource guard` waits through a bounded startup grace period, samples only
verified prefix-owned Wine/Windows Steam and arbitrary Windows game `.exe`
processes, and invokes the same prefix-scoped stop path when RSS exceeds the
limit. `status --json` attributes RSS by PID and category without emitting
paths or command arguments. At exit, timeout, or containment, the guard records
the peak category split in its log and creates an immutable JSON record under
`games/APPID/resource-guard-evidence/`. Evidence-write failure never
prevents threshold containment. It never uses a global `killall` or `pkill`.
Overlapping login, install, and launch commands reuse an existing guard only
when AppID, data root, runtime, and profile fingerprint all match; distinct
configurations retain separate guards. This prevents duplicate monitors from
accumulating after repeated commands.
Records created by a normal Steam launch are bound to the profile fingerprint,
runtime fingerprint, client mode, renderer, graphics API, and hash of the
active Steam session marker. Manually started or SteamCMD distribution guards
are explicitly labeled `unattributed-guard` and are not valid profile A/B
evidence. A later run cannot overwrite an earlier peak record.

The strict `resource compare` gate accepts only schema-3 records that observed
Windows Steam in both sessions (and a Windows game in `--phase game`). It also
requires distinct profile/session hashes, the exact requested Standard and
LowMemoryExperimental modes, identical runtime fingerprint, renderer, graphics
API, guard limit/cadence/terminal reason, and profile files which normalize to
identical content after changing only `steam_client_mode`. It also requires a
minimum observation duration, a phase-specific continuous Steam/game sample
run of that length, and similar durations/sample counts. Its optional output is create-new and hashes
both source records. A lower number is therefore reported only as a bounded A/B
result, not as proof that the game rendered, accepted input, or remained stable.
For an older prefix that predates the runtime identity marker, pass its verified
registered runtime as `--runtime-id ID`; normal managed launches supply this
identity to their guard automatically.

Shutdown signals remain restricted to the explicit Wine/Steam helper allowlist
after the exact-prefix `wineserver -k` request. Macroton then performs the wider
read-only `.exe` audit and fails rather than claiming containment if an
arbitrarily named game process survived.

Steam-mode `macroton launch` starts this guard automatically with a 4,096 MiB
soft limit and writes its observations to the game's `resource-guard.log`. It
starts an unattributed prelaunch guard before Wine Steam's cold
client warmup, then replaces it with a profile/session-bound guard after the
game handoff request. The early guard gives the first CEF startup the same RSS
limits; its samples are not valid game A/B evidence. A guard-triggered stop
also cancels an in-flight launch retry for the same AppID. By default, a 1.5×
hard limit stops an observed startup burst immediately; the
soft limit allows only a bounded number of consecutive excess samples. Use
`--max-rss-mib N --hard-rss-mib M` (M >= N) to set a tighter hard threshold.
An explicit hard limit samples once per second, but cannot prevent a transient
peak between samples. The explicit
`--no-resource-guard` escape hatch is intended for controlled diagnostics, not
normal play.

For a controlled launch-lifetime experiment, `macroton steam trim-client
APPID --profile PATH` submits the official Windows Steam `-shutdown` request
only after Steam's log reports a real running game PID and that PID is still
owned by the exact prefix and classified as a game process. The active
profile/runtime/session must match and Steam/CEF must be resident before the
request. It passes only if the game remains resident while Steam/CEF disappears
within 30 seconds, then creates an immutable redacted record under
`games/APPID/trim-client-evidence/` with before/after RSS and reduction. This is not a default optimization:
Steamworks, DRM, achievements and Cloud behavior must be revalidated per game,
and official Steam must be restarted after exit until Cloud reports `synced`.

After that restart, `macroton steam cloud-wait APPID --profile PATH` provides a
bounded post-exit gate. It requires explicit `cloud_files` in the profile and
returns success only when every declared local file either matches Steam's
synced raw cache hash directly, or matches decoded local remote-copy content
whose physical hash matches the cache. A historical Cloud log line or an empty
pending count alone is insufficient. Profiles without declared save paths fail
closed until their mapping is researched. This gate verifies only the local
Steam Cloud cache. `cloud-status` reports remote freshness separately: a
`pending-other-session` or `bypassed-pending-session` result keeps
`launch_ready=false` even when the local cache is `synced`. Reconcile any
save choice in Steam on the owning devices; Macroton never selects a version.
Even an `unverified` result cannot prove that another device has no newer data.
The only explicit exception is
`steam_cloud=unsupported`, accompanied by `steam_cloud_source` and
`steam_cloud_reviewed_date`, used when the reviewed official Steam feature list
does not advertise Cloud; it omits the network roundtrip but still requires human
in-game local save/reload. Release-profile audit rejects a profile that has
neither mappings nor this explicit policy.

A fresh upload/download claim uses the recoverable transaction command, not
`cloud-wait` alone:

```bash
macroton steam cloud-roundtrip begin APPID --profile PATH --data-root PATH
# Create a new save in the game, exit normally, let Steam upload, then stop it.
macroton steam cloud-roundtrip confirm-upload APPID --profile PATH --data-root PATH
macroton steam cloud-roundtrip quarantine APPID --profile PATH --data-root PATH
# Restart authenticated Steam so it downloads, then stop the prefix again.
macroton steam cloud-roundtrip verify-download APPID --profile PATH --data-root PATH
```

If an interrupted transaction is permanently obsolete and is still blocking
the profile, inspect it first with `cloud-roundtrip status`, then use the
explicit, recoverable cleanup form:

```bash
macroton steam cloud-roundtrip abandon APPID --profile PATH --data-root PATH --confirm
```

`abandon` is restricted to a `baseline` transaction, requires `--confirm`,
and moves the active marker to `transaction.abandoned`; it never deletes the
transaction evidence or Steam Cloud content.

The transaction requires an idle exact AppID prefix at every mutating step. It
proves the declared content changed after the baseline, observes a successful
upload after that checkpoint, moves both local saves and the AppID Cloud cache
to a recoverable quarantine, then requires a newly rebuilt cache, a later
successful download, and byte-identical restored content. `restore` moves the
quarantined saves/cache back only when doing so cannot overwrite current data;
`status` resumes an interrupted workflow. This does not itself prove the game
can load the restored save, which remains a separate in-game gate.

The Windows Steam desktop client is no longer the preferred download/update
engine: bounded tests show its CEF helpers can exceed that ceiling even with
chat, friends, and GPU UI flags disabled. Valve SteamCMD completed its own
update and an anonymous AppInfo query below about 305 MiB, so it is the current
distribution direction. It is not yet a replacement for the authenticated
Steamworks/Cloud process at launch time.

Start an explicitly interactive, per-AppID SteamCMD authentication session with
a lower default memory ceiling:

```bash
macroton steamcmd login APPID \
  --tool-root /private/path/to/windows-steamcmd \
  --runtime-id macroton-winehq-macos-11.14
```

At the `Steam>` prompt, enter `login ACCOUNT_NAME`; enter the password only at
SteamCMD's hidden prompt and complete Steam Guard there. Macroton deliberately
has no password option and never places one on the process command line. Keep
the tool root private because Valve may persist authenticated state there. The
command validates Valve's `steamcmd.exe` as a Windows i386 or x86_64 PE (the
bootstrap self-updates from i386 to x86_64) and starts a 500 MiB exact-prefix
resource guard before Wine executes it. Login-only sessions also have a
30-minute ceiling; reaching it stops only that AppID's exact prefix so an
abandoned credential prompt cannot remain resident indefinitely. Acquisition
downloads have no time ceiling but retain the memory ceiling.

The command must run from a real interactive TTY; Macroton refuses to start an
interactive login or acquisition from a background/non-TTY agent session. For
the supported visible-terminal path use:

```bash
scripts/open-steamcmd-login-terminal.sh APPID
```

If Terminal automation is unavailable, print a credential-free command for
manual execution with `scripts/open-steamcmd-login-terminal.sh --print-command
APPID`.

For a clean visible acquisition session, use the dedicated helper after the
account is authenticated:

```bash
scripts/open-steamcmd-acquire-terminal.sh APPID
```

It opens a fresh Terminal window with the profile-pinned `steamcmd acquire`
command and does not include credentials.

After authentication, acquire the profile-pinned Windows build into a dedicated
payload directory:

```bash
macroton steamcmd acquire APPID \
  --profile compatibility/experiments/APPID-preflight.conf \
  --tool-root /private/path/to/windows-steamcmd \
  --runtime-id macroton-winehq-macos-11.14
```

Macroton sets SteamCMD's platform to Windows and its install directory to
`games/APPID/steamcmd-payload`. At the prompt, log in, run
`app_update APPID validate`, wait for SteamCMD's fully-installed success line,
and enter `quit`. A successful command then requires the profile's real game
entrypoint to exist, validates it as PE, reports its architecture, and derives
the imported graphics API evidence. The explicit `--runtime-id` must equal the
profile-composed runtime ID before SteamCMD starts; a mismatched runtime is
rejected before credentials or downloads are requested. A clean SteamCMD exit without that payload
fails closed. On success, `steamcmd-acquisition.json` records the profile,
runtime, SteamCMD and game-executable fingerprints plus architecture and
graphics APIs without account data. It also requires SteamCMD's real
`appmanifest_APPID.acf`, atomically preserves a private snapshot beside the
schema-2 acquisition record, records its checksum, and compares its AppID,
BuildID and full
depot-to-manifest map exactly with the profile. Extra owned DLC is treated as
profile drift, so the observed installation must be reviewed and pinned rather
than silently accepted.

The read-only gate is available before, during, and after acquisition:

```bash
macroton steamcmd status APPID --profile PATH [--runtime-root PATH] [--json]
macroton steamcmd validate APPID --profile PATH [--runtime-root PATH] [--json]
```

It distinguishes `missing`, `partial`, `unrecorded`, `ready`, `drifted`, and
`invalid`. Only `ready` means the current profile, real PE entrypoint, resolved
runtime ID/fingerprint, preserved manifest checksum, BuildID and exact depot
manifests still match the recorded acquisition evidence; it does not claim
gameplay or Cloud success. Runtime or manifest-snapshot drift fails closed.
Machine-readable status includes `drift_reasons`, containing only the failed
categories among `marker-schema`, `profile`, `executable`, `runtime` and
`manifest`. Missing/partial acquisition reports an empty list because an
incomplete download is a lifecycle state, not provenance drift.
For `missing` and `partial`, status remains available even when the runtime
artifact has not been installed yet; runtime absence becomes a drift/blocker
only once acquisition evidence would otherwise be eligible for `ready`.

`validate` combines that acquisition gate with the per-AppID Windows Steam
bootstrap and prerequisite runtime states. Its schema-1 output distinguishes a
blocked acquisition, missing Steam client, unsigned installscript, first launch,
active prerequisite installation, and a later Steam-tracked game process.
`ready_for_launch` is deliberately narrower than “the game works”: stored login
metadata does not prove an online Steamworks session, gameplay, audio, input,
save persistence, or Cloud sync.

A `ready` acquisition can be fed into the normal Macroton planner without
pretending it came from the native macOS Steam library:

```bash
macroton steamcmd plan APPID --profile PATH
macroton steamcmd launch APPID --profile PATH --max-rss-mib 1600
```

Macroton constructs a bounded synthetic Windows installation record from the
verified acquisition evidence, then reuses the existing runtime, renderer,
payload-view, crash-guard, Steamworks launch, result-recording and resource
guard paths. Launch remains blocked until a separately verified Windows Steam
client is bootstrapped in that AppID prefix; SteamCMD replaces desktop Steam for
distribution, not for ownership, DRM, achievements, Overlay or Cloud.

The command resolves the runtime recorded in that prefix, always invokes its
`wineserver` with the exact `WINEPREFIX`, and checks for a strict allowlist of
Wine/Steam helper orphans that hold any open path inside that same canonical
prefix. This includes Steam.exe when its host-side working directory is
elsewhere but its image, DLLs, logs, or cache belong to the prefix. The
allowlist includes Wine-managed service processes such as
`services.exe`, `svchost.exe`, `rpcss.exe`, and `conhost.exe`. It sends TERM and,
after a bounded wait, KILL only after revalidating each exact PID. An unrelated
host process is excluded even if it has a file open in the prefix. There is no
global `pkill`, so other Wine and Macroton prefixes remain outside its scope.

The command does not acquire a Windows depot or bypass Steam authentication.

For a downloaded Windows entrypoint, `macroton inspect-pe PATH [--json]`
provides bounded static compatibility evidence: PE architecture, D3D/OpenGL,
XInput/DirectInput/SDL, XAudio/DirectSound/OpenAL/FMOD/Miles/SDL/WinMM, and
legacy DirectX/Visual C++/PhysX dependency hints. Acquisition records these
sets in its drift-checked marker. Static imports identify required surfaces;
they never count as runtime input, audible-output, or prerequisite-completion
passes.

`macroton steam prerequisite-status APPID [--json]` separates signed
installscript requirements from runtime completion. It reports the declared
`Run Process` entries and whether the installscript signature field is present,
then requires a later Steam `game process added` event before returning
`passed-to-game`. A static VC++/DirectX/PhysX hint, a signature field, or a
`RunningInstallScript` event alone is not completion.

## Windows Steam bootstrap

`steam bootstrap` turns client installation into a reproducible Macroton step.
It requires an explicit Valve installer plus its expected SHA-256, resolves the
runtime pinned by the AppID profile, and invokes the installer without a shell.
On success it writes `.macroton-steam-bootstrap.json` inside the prefix with
the installer and runtime fingerprints. It also writes the prefix runtime
identity before starting the installer, so a failed or interrupted setup can
still be stopped with the exact managed `wineserver`. Repeating the same
command is idempotent and repairs a missing runtime identity on older Macroton
bootstrap prefixes. It can also update only the bootstrap provenance when a
canonical record carries an older fingerprint for the same runtime ID and
installer, but only if the prefix is idle, its current runtime identity already
matches the resolved artifact, and Steam has zero stored logins and userdata
directories. This covers audited runtime-registry migrations without taking
over an authenticated prefix. Every other mismatch remains a hard refusal.

Inspect installation and sign-in readiness without starting Steam or exposing
account identifiers:

```sh
macroton steam prefix-status APPID
macroton steam prefix-status APPID --json
macroton steam login-wait APPID --timeout-seconds 600
```

`sign-in-required` means the managed client and bootstrap marker exist but no
stored login record was found. `stored-login-present` is only local metadata evidence; this
read-only command deliberately does not claim that Steam is currently online
or authenticated.

Start account-owner interaction as a separate bounded lifecycle:

```sh
macroton steam login APPID --profile compatibility/APPID.conf \
  --data-root /path/to/macroton-data --max-rss-mib 1000
```

`steam login` requires a structurally valid bootstrap record whose AppID,
installer/runtime SHA-256 values and runtime identity agree. It never copies or
accepts credentials. Its client-only plan deliberately does not require the
target game to be installed: authentication must be possible before Steam can
download the owned Windows depots. The later game `plan`/`launch` path still
requires the pinned BuildID, manifests and PE entrypoint. On macOS it fails
before Wine starts if `IOConsoleLocked`
is true or the lock state cannot be proven; ordinary Steam game launch has the
same interactive-session gate. On success it starts only Windows Steam using
the profile's fingerprinted client mode and attaches a four-hour exact-prefix
resource guard. For a fresh prefix, success requires ten continuous seconds of
real Steam client liveness plus either a newly created visible
`DesktopLoginWindow` marker in Steam's own webhelper log or verified growth in
Steam's current-run bootstrap update log. The latter is reported only as
`client-update-progress-observed`; it is never treated as a usable login UI.
Historical log entries, a non-progressing updater, or a bootstrapper-only
wineserver cannot satisfy this gate. The guard tolerates fewer than five consecutive five-second samples over
the configured RSS limit so Steam CEF can settle during startup, but stops the
prefix on a sustained excess or immediately above 150% of the limit. The game
client is also placed in its own Unix process group so returning from the
Macroton CLI cannot hang up the long-lived Wine/Steam process. The game is
launched separately after live authentication is verified.

Long-running guards sample at the configured cadence but keep normal logs
bounded: they report the first sample, process-count changes, RSS changes of at
least 16 MiB, and a five-minute heartbeat. Limit transitions and terminal peak
evidence are always reported, so quieter logs do not weaken containment.

Interactive login and Steam game launch also require an absolute persistent
data root. Macroton rejects `/tmp`, `/private/tmp`, macOS per-user temporary
folders and relative roots because cleanup would silently destroy the stored
login, userdata, Cloud cache and isolated prefix. The default
`~/Library/Application Support/Macroton` root is persistent. The explicit
`--allow-ephemeral-data-root` override exists only for disposable diagnostics;
it must not be used for account login or retained game evidence.

`prefix-status` reports `Bootstrap: valid` only after validating the marker's
schema, AppID, runtime kind and SHA-256 fields against
`.macroton-runtime-identity`. Merely creating a marker file, or retaining a
historical client with stored login metadata, is not bootstrap provenance.

After the owner completes the Steam UI or short-lived QR flow, `login-wait`
provides the strict acceptance boundary. It refuses to run while the Steam
client is still updating and succeeds only when the same persistent AppID
prefix simultaneously has a stored login, numeric account userdata, a live
primary Steam client, a connected transport, and a redacted nonzero account
context in the latest connection state. It never prints or stores the account
ID. Scanning a QR code alone is not success.
The runtime ID and fingerprint must also match the current launch plan.

For GUI and automation preflight, use:

```sh
macroton steam session-status --json
```

It sets `interactive_steam_allowed` only for a provably unlocked macOS console.
Locked and indeterminate sessions fail closed before Wine starts.

`steam inventory --with-appinfo` also prints a conservative `target-gate`.
An installed, current native Mac build is excluded; a Windows-only title or a
locally proven 32-bit Mac build with a Windows fallback is a candidate. Cached
dual-platform metadata without local binary evidence remains a manual native
viability review and is never promoted merely because the game is old.
If every cached Windows launch explicitly declares `osarch=64`, it is excluded
from the legacy target set. Missing architecture metadata is not Win32 proof;
the downloaded executable must still pass PE32/i386 inspection.
Use `--candidates-only` to apply these conservative exclusions before `--limit`
is applied. It implies `--with-appinfo`; metadata failures and native-viability
review entries stay out of the actionable queue instead of being guessed into
it.
For the current Macroton product scope, `--2d-only` additionally requires an
eligibility review with `scope=2d`; missing, `3d`, and `unknown` scope values
fail closed. This is a selection gate, not proof that the game is playable.
Combine it with `--verified-win32-only` to require an installed PE32 entrypoint
in the selected Steam root; JSON identifies that intersection as
`verified-win32-2d-only`. With explicit `--data-root`, inventory also checks
each listed title's **own** Macroton AppID prefix for installed PE evidence,
so scanning one prefix's account history need not hide verified Win32 payloads
in other isolated prefixes. This cross-prefix evidence also requires that
Steam's install-status gate confirm a completed manifest and nonempty payload;
a stray PE file or metadata-only manifest is insufficient. It never borrows a
different AppID's manifest or binary. `steam target-status` remains the
single-title native/Windows gate.

`steam launch-status APPID --json` is the redacted lifecycle interface for
automation and the future GUI. It reports Steam's AppID state, tracked game PID
and exit code without copying raw console-log lines or account-local paths into
the result. `running-observed` still requires a matching exact-prefix live PID
before any gameplay gate can pass.

`steam lifecycle-status APPID --json` combines the console gate, prefix
readiness, Steam launch event and exact-prefix live-process observation. A
historical `running-observed` log event without the same live PID is reported
as `stale-running-event`, never as a running game. During first-run client
download, a live primary `Steam.exe` plus a bootstrap update record modified
within five minutes reports `steam-client-updating`. A still-running client
whose update evidence has stopped changing reports `steam-client-update-stalled`,
not a login window. Only the post-update unauthenticated state reports
`login-ui-active`; helper-only CEF or `steamservice.exe` processes do not
satisfy any primary-client state. Inspect update progress directly with:

```sh
macroton steam client-status APPID --json
macroton steam client-wait APPID --timeout-seconds 14400
```

Long bounded waits automatically hold a macOS idle-system-sleep assertion for
their own process lifetime. This covers client updates, login verification,
game installation, Cloud reconciliation and normal-exit evidence watches; it
does not keep the display awake or change persistent power settings.

`client-status` derives an account-redacted recent KiB/min rate and ETA from a
15–120 second window of the updater's own timestamped byte checkpoints. It
never carries a rate across a restarted download, changed total, date boundary
or reversed counter. `client-wait` prints only changed progress, exits when
Steam records the latest observed update attempt as current, and fails on an explicit updater error,
unknown state, five-minute progress stall or bounded timeout. It does not infer
that the post-update login window is visible or that a later client launch
will not start another platform/update stage. Valve may complete a bootstrap
payload and then request a separate win64 client payload on the next launch.
Run `steam login` again; if it reports update progress, run `client-wait` again
without restarting that prefix. Only a post-update login UI or accepted CEF
target establishes that the client has converged far enough for authentication.

`evidence snapshot` atomically creates a new, redacted compatibility record and
refuses to overwrite an existing result. It binds the snapshot to the profile
fingerprint and records session, bootstrap/login checkpoint, Steam lifecycle,
exact-prefix RSS and declared Cloud content agreement. It explicitly lists
presentation, input, audible quality, save/reload, fresh Cloud roundtrip,
stability and normal exit as not inferred; those still require their real
runtime gates.

`evidence verify-audio` accepts the JSON emitted by
`tools/macos_audio_probe.swift` only while Steam's tracked game PID is still
live inside the exact AppID prefix and the macOS console is unlocked. It checks
non-silent PCM buffers, stable format, zero invalid buffers and window-owner PID
identity, then hashes both probe and profile into a non-overwriting record.
The prefix bootstrap must also match the selected installed runtime ID and
runtime fingerprint.
This is an audio-presence gate, not a listening/content/quality pass.

Steam launch profiles default to `steam_client_mode=standard`. The explicit
`low-memory-experimental` mode adds only the installed-client-verified
`-nofriendsui` and `-cef-disable-gpu` switches and is included in the profile
fingerprint and printed launch plan. It is not a universal default: lower CEF
RSS alone does not prove login, Steamworks, Cloud, game launch, or clean exit.
On Wine, disabling CEF GPU rendering can create a live Steam window whose
contents remain black. The `rendered-login-experimental` mode keeps
`-nofriendsui` but restores CEF rendering; it is the bounded fallback for an
interactive login, not a release default. A Steam UI log entry proves only
that a window was created, never that its pixels are visible or correct.
If that path reaches Steam's `Browser requested transparent background, but it
is not supported` diagnostic, `opaque-software-experimental` combines the
lower-RSS software path with Steam's own `-cef-force-opaque-backgrounds`
switch. It remains experimental until a human confirms visible pixels and the
login transport succeeds.
For an owner-assisted login when the macOS-composited surface remains black,
`cef-debug-experimental` exposes Steam's localhost-only DevTools endpoint inside
the same Wine prefix. Macroton may capture the legitimate short-lived QR page
from CEF itself; it must never persist or commit that QR, type credentials, or
claim authentication until Steam records the authenticated session.
`scripts/capture-steam-login-qr.sh APPID RUNTIME [DATA_ROOT] [WAIT_SECONDS]`
can wait a bounded interval for that unique page. It fails without preserving
an image when CEF is unavailable (including while Steam is still updating) or
when no unique sign-in target exists.
For Portal's independent Metal path, use
`compatibility/experiments/400-dxvk-cef-debug.conf`; it differs from the game
profile only in this client mode, so login diagnostics do not silently switch
back to the OpenGL runtime.
Unsupported switches such as `-no-browser` and `-cef-single-process` are not
used. Keep these profiles under `compatibility/experiments/` until a complete
unlocked-session game lifecycle passes.

After the isolated Windows Steam client is fully updated and has a stored
login, request an owned Windows depot using the numeric AppID. Macroton refuses
this action for an ephemeral prefix, an unfinished updater, a missing stored
login, a disconnected or unauthenticated account context, a missing live
primary Steam client, or a mismatched profile/runtime identity, and starts a
bounded resource guard with the request:

```sh
macroton steam install APPID --profile compatibility/APPID.conf
```

The command sends the Steam client protocol action
`steam://install/<numeric-appid>` to the Windows Steam client. It does not
download depots itself, bypass ownership checks, accept arbitrary protocol
URIs, or treat request submission as installation success.

Inspect the latest observed install attempt without keeping the Windows Steam
UI open:

```sh
macroton steam install-status APPID
macroton steam install-status APPID --json
macroton steam install-wait APPID
```

The command separates manifest checkpoint bytes, the reusable progress Steam
recognized at the latest attempt start, and attempt-scoped network bytes in
`content_log.txt`. It reports selected and committed depots and classifies
repeated zero-rate content-server failures as `stalled`. Its state is historical
evidence from Steam's files, not proof that the client is currently running.
Output is account-redacted and never includes raw log lines, URLs, or
authentication tokens. A literal `TargetBuildID` of zero in an older complete
manifest means there is no pending update; it is not treated as a real target.
`installed` additionally requires a nonzero manifest `SizeOnDisk` and a
non-symlink, nonempty `steamapps/common/<installdir>` directory. A complete-looking
synthetic/no-copy manifest with `SizeOnDisk=0` is `manifest-only`, even when its
payload link exists; `macroton plan` remains the authoritative check for an
external owned depot view. The versioned JSON form omits local paths and is
intended for the GUI and a future resource guard; it preserves the same
evidence boundary.

`steam install-wait` is the bounded orchestration form. It reports only changed,
account-redacted checkpoints, succeeds only when `install-status` proves a
nonempty, non-symlink payload with a completed manifest, fails immediately on
explicit failure or a classified network stall, and rejects a request that
does not appear within 60 seconds by default. Use
`--request-timeout-seconds N` when the visible Steam UI needs more time for
manual Install confirmation. When the fresh content log shows that
`steam://install/<appid>` was executed but no update has begun, the state is
`awaiting-confirmation`; this is the explicit point at which the visible Steam
Library still needs the user to click **Install**. Old content-log timestamps do
not produce this state.
If Steam then opens a game's legal agreement, a recent terminal UI-log prompt
is reported as `awaiting-eula`. Only the account owner may accept it; no
download or ownership/architecture conclusion follows from that state.
An older terminal prompt without a subsequent UI event is reported as
`eula-prompt-unconfirmed`, not as a fresh modal or a never-requested install;
check the live window before retrying.

For a clean prefix whose first official client update is prohibitively slow,
`scripts/seed-steam-public-client.sh` can reuse an already verified local Valve
client payload. The target must be an idle Macroton-bootstrap prefix with no
stored login or account userdata. The script excludes authentication, library,
Cloud, cache, log, crash, and host-network state; it records the copied public
manifest and executable hashes in `.macroton-steam-client-seed`. Steam must
still verify the client and perform its own login afterward. Prefix idleness is
checked from Wine's device/inode-derived server socket, so restricted process
visibility cannot be mistaken for an idle prefix.

Authentication is intentionally not imported. The account owner must sign in
through Windows Steam, after which `macroton launch` maps the owned depot into
that isolated library and requests `Steam.exe -applaunch APPID`.

Transport diagnosis is separate from both stored-login and live-auth evidence:

```sh
macroton steam connection-status APPID
# Machine-readable, account-redacted transport/login diagnostics:
macroton steam connection-status APPID --json
```

The command reports the latest connection-log state and whether the completed
endpoint falls in the `198.18.0.0/15` benchmarking range commonly used by proxy
Fake-IP modes. It redacts account fields and never promotes a transport
`connected` result to authenticated Steam ownership.
The JSON form also exposes `auth_poll_transport_error`, allowing a launcher to
distinguish a visible/login-page session whose QR polling is failing from a
successful post-login UI state.

Steam Cloud is handled by that authenticated Windows Steam client, not by the
native macOS Steam process. After exiting a game normally, inspect the actual
cache and latest upload/download result with:

```sh
macroton steam cloud-status APPID
```

Do not terminate the Wine Steam client while the command reports `syncing`.
When a profile declares `cloud_files`, pass it to the diagnostic:

```sh
macroton steam cloud-status APPID --profile compatibility/APPID.conf
```

Macroton then compares each Steam cache entry and its staged `remote` copy to
the explicitly rooted local game content by size and SHA-1. Legacy unqualified
paths and `user:` paths resolve below the Wine user; `steam-install:` paths
resolve below the profile's pinned Steam install directory; `steam-remote:`
paths resolve inside the selected account's AppID `remote` cache and are
quarantined/restored with that cache as one unit. Steam AutoCloud can store a
staged copy as a four-byte original length followed by zlib data; Macroton first
authenticates the physical object against `remotecache.vdf`, then compares its
decoded content to the local save. `synced` is reported only when the declared
local content matches; `local-drift` means Steam's cache is internally synced
but does not contain the current game save. This check is read-only and does
not overwrite or upload user data.

Steam launch submission is likewise not a compatibility pass. Inspect the
client's AppID lifecycle with:

```sh
macroton steam launch-status APPID
```

It distinguishes `submitted`, `installing-prerequisites`, `waiting-eula`, `waiting-user`,
`stale-waiting-user`, `running-observed`, `exited`, and `failed`. Its JSON
schema 2 includes `game_process_live`, so automation can reject a historical
`running-observed` event with no exact-prefix PID. The command is based on Steam's
console-log lifecycle; `running-observed` means Steam recorded a process start,
not that a separate host liveness probe currently sees that process. For an
`exited` launch, Macroton also reads Steam's `gameprocess_log.txt` and reports
the latest tracked process exit code when one is available.

## Near-term milestones

1. Turn the verified WineHQ-based runtime into a signed Macroton release.
2. Build the 2D-game compatibility matrix (SDL, GDI, DirectDraw/OpenGL,
   XInput/DirectInput, audio and common save locations) and promote only
   evidence-backed profiles.
3. Replace the host-specific Fake-IP diagnostic override with a general DNS
   preflight and remediation path.
4. Add automated main-menu evidence capture and compatibility result records.
5. Promote Heroes III and at least one additional eligible 2D Win32 title only
   after visible gameplay, input, audio, save/load, Cloud and stability gates.
6. Keep 3D experiments and frame-rate work outside the support matrix.
7. Add the SwiftUI shell after the 2D launch/evidence path is repeatable.

See [docs/architecture.md](docs/architecture.md) for boundaries and design
decisions.

The pinned clean WineHQ build recipe is documented in
[docs/runtime-build.md](docs/runtime-build.md).

The first real-game target and its evidence checklist are documented in
[docs/first-playable-silver.md](docs/first-playable-silver.md).

The second Win32 runtime result and its multi-depot finding are documented in
[docs/evidence/400-main-menu-2026-08-12.md](docs/evidence/400-main-menu-2026-08-12.md).

The repeatable evidence levels and test commands are documented in
[docs/testing.md](docs/testing.md).
