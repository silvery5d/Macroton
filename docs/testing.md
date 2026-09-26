# Testing Macroton

Macroton uses four evidence levels. Passing a lower level never implies a
higher one.

## Candidate eligibility gate

Before downloading a Windows depot, verify the game actually needs Macroton on
the target macOS release. A candidate is eligible only when all of these are
true:

1. the account owns the Steam game;
2. there is no official macOS build that currently runs on the target Mac, or
   the available official Mac build is demonstrably unusable there (for
   example, it is 32-bit-only);
3. the owned Windows build contains a real PE32/i386 game entrypoint;
4. the initial test does not depend on unsupported anti-cheat or a prohibited
   DRM workaround.

The gate is ordered: check current native macOS viability before downloading
or testing the Windows depot. A working official Mac build excludes the game
from Macroton's legacy-recovery validation set even if its Windows build would
also run under Wine. `Baba Is You` is the reference exclusion for this rule;
`Portal` remains eligible because its official Mac release is 32-bit-only on
modern macOS.

For a two-root, fail-closed selection result, use:

```sh
macroton steam target-status APPID \
  --native-steam-root /path/to/native/Steam \
  --windows-steam-root /path/to/authenticated/windows/Steam \
  [--eligibility-review eligibility/APPID.conf] --json
```

The native root decides whether an official Mac build already works; the
Windows root independently proves a path-bound PE32 game entrypoint. A working
native build always excludes the game, even if its Windows package is also
PE32. Missing native metadata returns `review-native-mac-viability`, not an
eligible result. Schema 3 also returns the observed native gate separately
from the effective gate, a coarse `stage`, a stable
`next_action`, and a human-readable `reason` for the launcher. `eligible` means
only that the title fits Macroton's recovery scope; it never means the game has
passed rendering, input, audio, save, Cloud, performance, or stability tests.

For inventory-first target selection, load the same review directory and
require a real path-bound PE32 game entrypoint:

```sh
macroton steam inventory --steam-root /path/to/authenticated/windows/Steam \
  --eligibility-dir eligibility --verified-win32-only --json
```

Schema 2 reports `selection=verified-win32-only` and includes each applied
review's SHA-256 and official source. A review marked `working` excludes the
title even if a PE32 Windows executable exists. A legacy review never erases an
already verified Win32 entrypoint. Any title whose cached metadata advertises
macOS must have a matching non-working review; missing review fails closed and
keeps it out of strict results. This strict mode removes metadata-only and
explicit 64-bit candidates; it still does not imply runtime compatibility.

When local AppInfo is stale or incomplete, a tracked eligibility review may
resolve it using the matching official Steam store AppID URL. The strict file
records `working`, `legacy-unusable`, or `windows-only`, its review date and a
stable evidence code; its SHA-256 and source appear in schema 3 output. A review
cannot override a locally proven working native build and cannot bypass the
exact path-bound PE32 Windows entrypoint gate.

For Macroton's current product scope, add `scope=2d` to the review and use
`--2d-only`. This is fail-closed: reviews without an explicit 2D scope, plus
3D and unknown titles, are excluded before runtime validation.

## 1. Core checks

```sh
cargo fmt --check
cargo test
cargo clippy --all-targets -- -D warnings
scripts/test-macos-swift-tools.sh
```

These validate parsers, path containment, runtime verification, isolated Steam
library mapping, Overlay policy composition, and launch-result semantics. They
do not start Steam or a game.

### Visible 2D input probe

The Swift interaction probe must be used only with a WindowServer window ID and
PID that were just resolved for the exact managed game process. Mouse actions
are posted directly to that validated PID, so a visible Wine window may be
tested even when another application is reported as macOS frontmost. Keyboard
actions still require the game PID to be frontmost. The probe captures before
and after frames and rejects a stale window, changed ownership, or a missing
pixel change; it is evidence for input/state change, not a substitute for a
human gameplay review.

## AoE3 Win32 CD-key dependency

Age of Empires III (AppID `105450`) ships the native `PidGen`/`MGSPID` DLLs,
but the PID component requires the legacy `MFC42.DLL` runtime. A fresh
Macroton prefix must install that dependency before the first launch:

```sh
scripts/install-aoe3-win32-dependencies.sh 105450 \
  "$HOME/Library/Application Support/Macroton" \
  "$HOME/Library/Application Support/Macroton/runtimes"
```

The script is idempotent and fails unless the exact AppID prefix contains
`mfc42.dll`, Core Fonts `cour.ttf`, and the legacy DirectPlay/codec/media
payloads `dplayx.dll`, `l3codecx.ax`, `quartz.dll`, and `dsound.dll` under
`drive_c/windows/syswow64`. MFC42 addresses the game's misleading “Error
loading the PID generator DLL” dialog; Core Fonts supplies the legacy Courier
New face used by the CD-key/UI path. The additional Winetricks verbs make the
old DirectPlay, MPEG Layer-3, Quartz and DirectSound dependencies explicit
and auditable rather than relying on whatever a host prefix happened to have.
The `105450` profile also forces the depot's native `pidgen.dll` and
`pidgenx.dll` because Wine's builtin `pidgen.dll` is only a stub. The script
additionally installs the pinned Microsoft .NET 2.0 SP2 x86 bootstrapper and
verifies the `Wow6432Node` v2.0.50727 registration; its Windows reboot-required
exit code `236` is treated as successful. It does not bypass Steam ownership
or validate a retail key.

The AoE3 profile intentionally does not pass `-windowed`: an isolated Wine
diagnostic reproduced an early `0xC0000005` exit with that argument, while the
same executable remained alive without it. Macroton now warms the authenticated
Steam client before submitting `-applaunch`; that handoff creates the game
process reliably. This is still not proof of stable in-game 3D play: the
2026-08-30 map-load run entered a high-CPU Wine exception loop. The independent
`compatibility/experiments/105450-legacy-media.conf` profile is reserved for a
controlled dependency A/B test.

For Steam-to-game race diagnostics, set
`MACROTON_STEAM_GAME_WARMUP_SECONDS=15` (accepted range 1–120) before
`macroton launch`. The default is 90 seconds, based on observed Wine CEF cold
starts of 69–82 seconds on the validation host; the override is bounded and
is intended for repeatability experiments, not as a claim that waiting alone
fixes a game crash.

After a launch, `macroton steam launch-status APPID --json` reports a fresh
`LogonFailure`/authentication error as `failed` (rather than `no-evidence`),
so an offline Wine Steam session is distinguishable from a game that crashed
after `Game process added`.

When the menu opens but selecting a campaign or skirmish map exits, use the
separate `compatibility/experiments/105450-mapload-seh.conf` profile once. It
enables `+seh,+loaddll,+timestamp` only for fault attribution and can produce a
large `steam-client.log`; it is not a normal-play profile. Record the exact
exit code and the first `Unhandled` exception after reproducing the same map
load, then stop the AppID prefix and compare with the default profile. A menu
or loading screen alone is not successful gameplay evidence.

To extract the first fault from a bounded diagnostic log without manually
scanning the full file, use:

```sh
scripts/extract-wine-exception.sh \
  "$HOME/Library/Application Support/Macroton/games/105450/steam-client.log" \
  "$HOME/Library/Application Support/Macroton/games/105450/.macroton-launch-diagnostic-offset"
```

The helper reports `unhandled_count`, `first_unhandled`, and the first line
containing an exit code. It accepts either Macroton's boundary file or a raw
byte offset; without a boundary it intentionally reports the complete log and
must not be used as fresh-run evidence.

For WindowServer presentation A/B, use
`compatibility/experiments/105450-cpu4-standard.conf`. It keeps the four-CPU
limit but switches the Steam client from opaque experimental CEF to `standard`.
This is useful when the Wine Steam surface is alive but reported in a negative
WindowServer coordinate on another physical display/Space; it does not itself
repair a game crash or establish gameplay.

## 2. Runtime checks

```sh
cargo run -- runtime verify \
  runtimes/macroton-winehq-macos-11.14.conf \
  /path/to/wine-devel-11.14-osx64.tar.xz

cargo run -- runtime probe-win32 \
  "/path/to/Wine Devel.app/Contents/Resources/wine/bin/wine"
```

The first command proves artifact identity. The second proves that the runtime
can execute an i386 PE payload through WoW64. Neither proves Steam networking or
game compatibility.

## 3. Read-only game plan

```sh
cargo run -- scan
cargo run -- steam library-status APPID --steam-root /path/to/windows/Steam
cargo run -- steam install APPID --profile compatibility/APPID.conf
cargo run -- steam install-status APPID --steam-root /path/to/windows/Steam
cargo run -- steam install-status APPID --steam-root /path/to/windows/Steam --json
cargo run -- steam install-wait APPID --timeout-seconds 14400
cargo run -- steam inventory --steam-root /path/to/windows/Steam \
  --min-playtime 60 --limit 50 --with-appinfo
cargo run -- steam inventory --steam-root /path/to/windows/Steam \
  --min-playtime 60 --limit 50 --candidates-only
cargo run -- steam appinfo APPID --profile compatibility/APPID.conf \
  --steam-root /path/to/windows/Steam
cargo run -- plan 606680 --profile compatibility/606680.conf
```

`steam library-status` can establish private, account-redacted historical
inventory evidence without claiming current entitlement. `steam appinfo` may
establish a cached Windows launch path, BuildID, public depot manifests and
Valve-declared UFS save/root-override rules before installation, but explicitly
does not prove entitlement, payload presence, local save presence or completed
Cloud synchronization. A title with no UFS rules may still use API-backed Steam
Remote Storage; require authenticated AppID cache/content evidence before
declaring `steam-remote:` paths. The plan must identify Windows depot `606681`,
`silver.exe`, the pinned runtime, and no blockers. A preflight-only candidate
such as Dishonored must remain blocked until the authenticated client writes its
real `appmanifest_205100.acf` and files. Planning does not mutate a prefix or
launch anything.

For the low-memory distribution path, first complete `macroton steamcmd login`
in the foreground. Then run `macroton steamcmd acquire APPID --profile PATH`
with the same private tool root, registered runtime, and data root. Macroton
preselects Windows content and an isolated payload directory; credentials are
entered only into SteamCMD. Verify that a `--runtime-id` differing from the
profile is rejected before tool validation or process startup. After
`app_update APPID validate` succeeds and the
user types `quit`, the command fails closed unless the profile-pinned game
entrypoint is a real PE and reports its architecture and imported graphics API.
This executable gate does not replace the subsequent manifest, launch, input,
audio, save, Cloud, or stability gates. Preserve the generated redacted
`steamcmd-acquisition.json`; its hashes make payload, runtime, tool and profile
drift explicit in later regression checks. Successful acquisition writes a
schema-2 record and an atomic private `appmanifest_APPID.acf` snapshot. Status,
validate, plan and launch must recheck that snapshot's AppID, BuildID, complete
depot map and checksum plus the currently resolved runtime ID/fingerprint.

An interactive login prompt is bounded to 30 minutes by the same exact-prefix
guard. Confirm that timeout cleanup affects only the AppID prefix. Acquisition
is intentionally not time-limited because a legitimate depot download can take
hours; its RSS ceiling remains active.

Run `macroton inspect-pe ENTRYPOINT --json` after acquiring a real executable.
Use the reported input, audio and prerequisite families to choose runtime tests
and DLL policy; do not promote them as functional evidence. The implementation
is cross-checked against real Alan Wake, Braid and Silver PE32 payloads in
`docs/evidence/pe-compatibility-inspection-2026-08-13.md`.

Run `macroton steam prerequisite-status APPID --json` around the first Steam
launch. `signature-missing` fails closed; `pending` and `installing` remain
incomplete. Only `passed-to-game` proves Steam moved beyond the signed
installscript into a real tracked game process. It still does not prove each
third-party installer independently succeeded or that gameplay works.

Acquisition additionally requires SteamCMD's `appmanifest_APPID.acf`. Its AppID,
BuildID and complete `InstalledDepots` manifest mapping must exactly match the
profile. Do not discard extra depots as irrelevant: an owner-specific DLC set
is observable installation drift until reviewed and pinned in an updated
experimental profile.

Use `macroton steamcmd status APPID --profile PATH --runtime-root PATH --json`
as the machine gate.
`missing` and `partial` are expected while acquisition has not started or is in
progress; `unrecorded` means a real PE exists without a completed marker;
`drifted` means profile, executable, runtime or manifest-snapshot evidence
changed; `invalid` means the entrypoint is not PE. Only `ready` passes acquisition integrity, and even that
does not pass manifest, gameplay, Cloud, or stability validation.
Assert the exact `drift_reasons` category rather than parsing the human detail
string. `missing`, `partial`, `unrecorded` and `invalid` do not fabricate a
provenance category that was never eligible for comparison.

These `steamcmd` commands apply only to profiles whose
`windows_payload_root=steamcmd-payload`. A Steam-installed-tree profile, such
as Heroes III, is validated with `macroton steam install-status APPID --json`;
running `steamcmd status` for it correctly reports no SteamCMD payload even
when the separate Steam depot is installed.

Use `macroton steamcmd validate APPID --profile PATH --runtime-root PATH --json`
for the aggregate
prelaunch view. It evaluates acquisition first, then the local Steam bootstrap,
then installscript integrity and runtime transition. A true `ready_for_launch`
does not satisfy the later gameplay, input, audio, save, Steamworks, Cloud, or
stability gates.

For runtime audio-presence evidence, first identify the exact game window ID and
owning PID, then run a bounded ScreenCaptureKit sample while performing a known
sound-producing game action:

```sh
swiftc tools/macos_audio_probe.swift -o /private/tmp/macroton-audio-probe
/private/tmp/macroton-audio-probe WINDOW_ID PID 10 -50
```

Retain the JSON output with the launch plan/runtime fingerprint. A non-zero
buffer count and non-silent ratio prove only that the filtered application/window
stream delivered PCM above the chosen threshold. They do not prove correct
content, balance, spatial audio, dropout-free playback, current output volume,
or human audibility. Complete the audio gate only after a listening pass tied to
the same known in-game action. Never substitute PE imports or static middleware
strings for this runtime evidence.

While the same Steam-tracked game PID remains live, bind the probe to Macroton's
profile and exact-prefix lifecycle:

```sh
macroton evidence verify-audio APPID --profile compatibility/APPID.conf \
  --data-root /path/to/macroton-data --probe /absolute/path/audio-probe.json \
  --runtime-root /path/to/macroton-runtimes \
  --output /absolute/path/new-audio-evidence.json
```

The verifier refuses locked sessions, stale Steam events, PID mismatch,
profile/runtime/client-mode session drift, silence, invalid buffers, format
changes and output replacement. Switching Standard/LowMemory or renderer while
Steam is active requires stopping the exact AppID prefix first. Its
`pcm-presence-pass` still requires a separate human listening/action check.

For focus-sensitive presentation capture, compile
`tools/macos_focus_stream.swift` with `swiftc -warnings-as-errors`, capture the
exact game window/PID at the intended rate, and preserve stdout as CSV. Verify
it while that same Steam-tracked PID remains live:

```sh
macroton evidence verify-focus-stream APPID --profile PATH \
  --data-root /path/to/macroton-data --runtime-root /path/to/macroton-runtimes \
  --probe /absolute/path/focus-stream.csv \
  --min-frames 600 --min-duration-seconds 9 --max-gap-ms 100 \
  --black-luma 1 --max-black-frames 0 \
  --output /absolute/path/new-focus-evidence.json
```

Every CSV row now includes the exact WindowServer ID and PID. The sampler
excludes the native title-bar chrome from `average_luma` and the sampled hash,
so a white macOS title bar cannot make an otherwise black Wine game surface
pass the black-frame gate. The verifier
rejects reordered/missing frames, unfocused samples, identity drift, invalid
luma/hash data, stale sessions and threshold failure. A pass rules out only
whole-frame black samples under the declared luma threshold and capture bounds;
it does not rule out localized or sub-sample flicker and does not establish
input response or acceptable gameplay FPS.

For an intentionally moving 3D scene, derive a bounded presentation-cadence
proxy from the same CSV while the exact PID remains live:

```sh
macroton evidence verify-motion-rate APPID --profile PATH \
  --data-root /path/to/macroton-data --runtime-root /path/to/macroton-runtimes \
  --probe /absolute/path/focus-stream.csv \
  --min-duration-seconds 10 --min-changed-rate 20 \
  --max-unchanged-ms 200 \
  --output /absolute/path/new-motion-rate-evidence.json
```

The proxy counts transitions in sampled whole-window hashes, reports delivered
callback rate, change rate, longest unchanged interval and p95 change interval.
It fails on a stale session, PID drift, insufficient duration/change rate,
excessive freeze interval or output replacement. It is not suitable for static
menus and does not infer engine FPS, simulation rate, latency or playability.
The target scene and expected motion must be recorded alongside the evidence.

For an input-associated frame change, compile the integrated probe and choose a
known static menu action. It refuses an existing output directory:

```sh
swiftc -warnings-as-errors tools/macos_interaction_probe.swift \
  -o /private/tmp/macroton-interaction-probe
/private/tmp/macroton-interaction-probe WINDOW_ID PID \
  /absolute/path/new-capture 500 key escape \
  > /absolute/path/interaction.json
macroton evidence verify-interaction APPID --profile PATH \
  --data-root /path/to/macroton-data --runtime-root /path/to/macroton-runtimes \
  --probe /absolute/path/interaction.json \
  --capture-dir /absolute/path/new-capture --min-changed-ratio 0.01 \
  --output /absolute/path/new-interaction-evidence.json
```

The probe also accepts `click X_FRACTION Y_FRACTION`, with both fractions in
the exact target window's 0--1 coordinate range. Macroton re-hashes `before.png`
and `after.png`, verifies the live Steam PID/session, focus/ownership, dimensions,
sample counts and changed ratio. Use a static menu and inspect both images:
animation can create an associated frame change without proving that the event
caused or semantically completed the intended action.

While the real game PID remains Steam-tracked, capture the runtime-independence
gate once per promoted profile/runtime combination:

```sh
macroton evidence verify-independence APPID --profile PATH \
  --data-root /path/to/macroton-data --runtime-root /path/to/macroton-runtimes \
  --output /absolute/path/new-runtime-independence-evidence.json
```

The verifier inspects current `lsof` paths for that exact host PID. Absence of a
forbidden name is insufficient: at least one mapped/open path must belong to the
selected managed runtime and at least one to the exact AppID prefix. Any path
containing CrossOver or CodeWeavers fails the gate. The evidence redacts paths
to counts plus a sorted-set hash and is explicitly only a point-in-time process
snapshot; repeat it for materially different renderers or runtime components.

For sustained-run and normal-exit evidence, start the watcher while the same
Steam-tracked game PID is live, then play and quit through the game's own UI:

```sh
macroton evidence watch-exit APPID --profile PATH \
  --data-root /path/to/macroton-data --runtime-root /path/to/macroton-runtimes \
  --min-runtime-seconds 1800 --timeout-seconds 14400 \
  --output /absolute/path/new-normal-exit-evidence.json
```

The watcher fails on lock/session/profile/runtime drift at startup, PID loss
without a Steam removal record, nonzero or missing exit code, insufficient
observed duration, timeout, or evidence replacement. A pass proves the tracked
game survived the measured interval and exited with code zero; it deliberately
reports remaining exact-prefix Steam/helper RSS rather than claiming those
processes exited. Follow it with `macroton stop APPID` and a zero-process
`resource status` check.

For a real validation run, the six live automated artifacts should be captured
from one Steam session and one tracked PID. After launching the game, run:

```sh
scripts/capture-runtime-evidence-session.sh APPID compatibility/APPID.conf \
  auto auto /absolute/new/evidence-dir /path/to/data /path/to/runtimes \
  key escape
```

The wrapper prompts for a moving scene, a known audio action, a reviewed static
menu input, and finally an in-game normal exit. It refuses locked sessions,
existing output directories, stale lifecycle/PID bindings and verifier failure.
The second `auto` takes the tracked PID from the current Steam lifecycle. The
first `auto` selects a window only when that PID owns exactly one on-screen layer-0
window of at least 320x180; when there are multiple candidates it prints their
IDs and dimensions and requires an explicit choice rather than guessing.
On failure, interruption or success it stops the exact AppID prefix and requires
both zero residual processes and zero RSS. Stop failure or remaining resources
make cleanup fail explicitly instead of silently claiming containment. Use environment overrides such as
`MACROTON_MIN_RUNTIME_SECONDS` only when declaring a different test protocol;
the release stability default is 1,800 seconds.

Some WineHQ macOS builds expose a real game as a WindowServer fullscreen layer
above layer 0. The capture helpers accept nonnegative layers for this case, but
they still require one unique on-screen window owned by the exact game PID. A
frame capture alone is not motion, input, audio, or gameplay evidence.

Steam may also pause at `ShowInterstitials / waiting-user` before creating the
game process. Cloud-enabled titles can instead report
`SynchronizingCloud "pendingcloudsessions"`; Macroton exposes this as
`waiting-cloud` while the Steam client is live and `stale-waiting-cloud` when
only an old console-log event remains. A `waiting-user` event is likewise
reported as `stale-waiting-user` after the exact-prefix client exits. Resolve
the Cloud prompt in the same
isolated Steam prefix before retrying; never delete local saves to clear it.
For a known, already authenticated session, a single Return to
the exact Windows Steam client can continue that interstitial; do not send
blind input to credential, EULA, or unexpected prompts. The Steam console log
must then show `Game process added` for the profile executable before the PID is
eligible for evidence collection.

The lifecycle command applies the same evidence boundary to Steam client
updates: a byte count from an old `bootstrap_log.txt` is reported as stale
historical progress unless the exact AppID Steam client is live and the update
log has changed within the freshness window. This prevents an abandoned
installer from being mistaken for an active acquisition.

Once acquisition is `ready`, run `steamcmd plan`. It must synthesize only the
verified Windows installation fields and pass the ordinary profile/runtime/
renderer planner. It must still report a blocker if the AppID prefix lacks a
Macroton-recorded Windows Steam bootstrap. `steamcmd launch` reuses the normal
Steamworks launch and 1,600 MiB exact-prefix guard; it does not let SteamCMD
stand in for Cloud or DRM.

`steam install-status` is the read-only download gate. It keeps Steam's
manifest checkpoints, attempt-start reusable progress, and bytes observed on
the latest content-log attempt distinct. It may report `stalled` after repeated
connection failures at zero rate even while the manifest says zero bytes, or
`queued` after a newer interrupted attempt supersedes those historical errors.
The result is a latest-observed file state, not live-process evidence.
When a fresh content log records `ExecuteSteamURL: "steam://install/APPID"`
but no update has started, the state is `awaiting-confirmation`; this means the
visible Steam Library still needs the user to confirm **Install**, not that the
URI was dropped. Old content logs are ignored for this state and for live
download states.
`awaiting-eula` means a recent app-specific Steam UI log ends with an EULA
prompt. It is a short-lived diagnostic, not proof of acceptance or download;
the account owner must decide whether to accept the agreement.
After ten minutes without a newer UI-log event, this becomes
`eula-prompt-unconfirmed`: the old prompt is known, but the current window and
owner choice are unknown. `install-wait` stops at that boundary instead of
claiming that no install was requested or silently waiting for hours.
`installed` requires a nonzero `SizeOnDisk` plus a non-symlink, nonempty
declared common-directory;
a metadata-complete synthetic/no-copy manifest is instead `manifest-only`.
That state does not reject a separately owned depot: use `macroton plan` against
the payload-discovery Steam root to verify its exact executable, build, depots,
renderer and runtime. Do not use a clean prefix's synthetic appmanifest as the
payload authority.

If `install-wait` remains `not-requested` for its 60-second request window (or
the configured `--request-timeout-seconds` value), the protocol handoff
was not accepted by the Windows Steam startup path. Open the visible Wine Steam
client for that exact AppID prefix, choose **Install** in its Library UI, and
then rerun `macroton steam install-wait APPID --request-timeout-seconds 300`; do not infer acquisition from a
successful Steam.exe exit or from historical content-log bytes.

For translated-game CPU diagnostics, summarize a saved macOS `sample` report
without starting Wine again:

```sh
scripts/summarize-macos-sample.py /path/to/game.sample.txt \
  --thread 'RMD - Game Thread' --json
python3 -m unittest tests/test_summarize_macos_sample.py
```

The marker counts are sums of top-level thread branches containing each string.
They can overlap and cannot identify unsymbolized PE32 guest functions; use them
to separate observed translated execution, Wine waits, and renderer calls, not
as source-level profiling.

Use `steam inventory` to choose the next owned candidate without emitting
account directory names or ticket content. Its output is not an eligibility
pass: check Valve's current platform metadata first, then require a real PE32
game executable rather than accepting a launcher process. `--with-appinfo`
annotates cached names and launch platforms as a fast, deliberately stale-safe
rejection hint.

## 4. Live Steam/game gate

After installing the pinned runtime and bootstrapping Valve's installer with a
separately verified SHA-256:

```sh
cargo run -- steam prefix-status 606680 --data-root /path/to/macroton-data
cargo run -- steam connection-status 606680 --data-root /path/to/macroton-data
cargo run -- launch 606680 --profile compatibility/606680.conf
```

The first command is a read-only readiness check. A new isolated prefix must
report `sign-in-required` until the account owner signs in through its own
Windows Steam client. A stored login record is not live-authentication proof;
the connection and AppID lifecycle logs remain authoritative for a live run.
The connection command independently reports Steam transport state and flags a
completed `198.18.0.0/15` Fake-IP route. It does not infer account login from a
working socket.

For Steam mode the command returns after submitting the request. A successful
result record has `launch_mode: "steam"` and `exit_code: null`; it also records
the selected `renderer`, declared `graphics_api`, and static
`graphics_evidence`. These fields establish launch provenance but are not yet
a game pass. Record all of the following:

1. Steam connection log shows a legitimate logged-on account.
2. `macroton steam launch-status APPID` advances past `submitted`,
   `waiting-eula`, `waiting-cloud`, and `waiting-user`; Steam console log shows the AppID and
   real game process. `running-observed` alone is a lifecycle observation, not
   a current host-process liveness guarantee.
   Wine graphics probes such as `gldriverquery64.exe` are classified as
   helpers, not `windows-game`; their presence cannot satisfy this gate.
   When a profile-backed Steam launch starts its guard, Macroton also passes
   the profile executable and requires the observed `windows-game` basename to
   match it. Generic resource guards remain useful for client/bootstrap work,
   but cannot be promoted as game-start evidence.
3. The process loads `steam_api.dll` and no disabled Overlay DLL.
4. WindowServer exposes a visible window titled `Silver`.
5. A capture shows the main menu.
6. A click opens another menu, with before/after captures and SHA-256 hashes.
7. After creating or updating a save, exit the game normally and wait until
   `macroton steam cloud-status APPID` reports `State: synced`. The cache must
   list the expected save files with no unexplained pending entries. Closing or
   killing the Wine Steam client before this completes can lose the upload.

The current reference record is
[`docs/evidence/606680-main-menu-2026-08-11.md`](evidence/606680-main-menu-2026-08-11.md).

For D3D10/D3D11 candidates, the open-source DXMT experiment can be composed and
probed without CrossOver:

```sh
scripts/compose-dxmt-runtime.sh \
  /path/to/wine-root \
  /path/to/dxmt-v0.74-builtin.tar.gz \
  /path/to/new-wine-dxmt-root

scripts/probe-dxmt-d3d10.sh /path/to/new-wine-dxmt-root/bin/wine
```

The composer verifies the pinned official DXMT archive, refuses to overwrite
an existing destination, and writes hashes for every injected component.
External runtime resolution rechecks those hashes. A successful probe proves
only that the Win32 DXMT modules and Unix `winemetal.so` load and create a
D3D10 device; it is a prerequisite for, not a substitute for, same-scene game
performance and rendering evidence.

Renderer selection is also API-gated. Each profile declares `graphics_api`;
Macroton rejects a DXMT runtime for a D3D9 game before launch. Alan Wake is the
reference regression case: `AlanWake.exe` imports `d3dx9_43.dll`, and its
renderer DLL imports `Direct3DCreate9`, so a successful D3D10 DXMT probe says
nothing about that game's path.

For an upstream DXVK D3D9 candidate, test a standalone PE32 device before any
game prefix is modified:

```sh
scripts/probe-dxvk-d3d9.sh \
  /path/to/plain-wine/bin/wine \
  /path/to/dxvk-3.0.2.tar.gz
```

The script pins the official archive SHA-256 and loads its native `d3d9.dll`
beside the probe. On the current WineHQ 11.14/MoltenVK 1.4.0 stack, DXVK 3.0.2
finds the Apple M4 Pro but rejects it because the required Vulkan
`geometryShader` feature is absent. Treat this as a fail-closed backend veto,
not as a game launch failure and never inject that DLL into Alan Wake.

The D3D9-to-D3D11 wrapper experiment is similarly isolated:

```sh
scripts/probe-dgvoodoo-d3d9-dxmt.sh \
  /path/to/wine-dxmt/bin/wine \
  /path/to/dgVoodoo2_87_3.zip
```

The probe pins the author's complete v2.87.3 release archive and copies only
its x86 D3D9 wrapper into a temporary probe directory. On the current DXMT
v0.74 stack it loads dgVoodoo's native `D3D9.dll`, DXMT's
`DXGI/D3D11/winemetal`, detects the M4 Pro, and selects D3D11 feature level
11.0, but then page-faults inside dgVoodoo while creating the D3D9 device.
This proves the intended chain was exercised and rejected before touching a
game. dgVoodoo's redistribution terms also prohibit bundling it as a generic
launcher/framework component, so it cannot become a hidden global Macroton
runtime dependency.

Macroton also records a runtime identity in every prefix. A different runtime
may replace it only while the wineserver is idle; an active unowned or
mismatched wineserver blocks launch. This prevents a command submitted through
one Wine entrypoint from silently reusing another runtime's server and creating
false renderer comparisons. Bootstrap writes that identity before invoking
SteamSetup and repairs it on an idempotent rerun. `macroton stop` retains a
legacy fallback to the matching bootstrap marker for prefixes created before
this invariant was added.

An idempotent bootstrap may repair a stale provenance fingerprint (including a
supported external-to-managed runtime-kind migration) only when the old marker
is canonical and otherwise matches AppID, runtime ID and installer SHA-256;
the prefix identity must either already match the resolved runtime or exactly
match the old canonical marker. The exact prefix must be idle, and both
stored-login and numeric userdata counts must be zero. In the second case the
identity is committed first and the bootstrap marker second, so interruption
leaves a state the idempotent repair can finish. Any divergent identity,
authenticated/userdata prefix or active process fails closed. It must not rerun
SteamSetup on this path.

Every launch also installs a fail-closed post-mortem debugger guard in the
isolated prefix's 32-bit and 64-bit `AeDebug` registry views. Wine defaults to
`winedbg --auto`; a graphics failure that also prevents the debugger from
attaching can otherwise produce a recursive process storm. Macroton replaces
that command with a deliberately absent executable, records the guarded runtime
fingerprint, and refuses to launch if either registry write fails. This changes
only Macroton's per-game prefix and does not modify host Wine installations.

Verify the behavior with a controlled PE32 crash:

```sh
scripts/verify-wine-crash-guard.sh \
  /path/to/wine \
  /path/to/macroton-data/games/APPID/prefix
```

For interactive tests that need the Windows Steam client, run an exact-prefix
resource guard in a second terminal. The standalone guard defaults to a
conservative 1,600 MiB test-host ceiling; the higher-level `macroton launch`
command uses a 4,096 MiB default because Wine Steam's CEF bootstrap routinely
peaks above 2 GiB on Apple Silicon. Both are explicit limits, not evidence that
the game itself uses that amount:

```bash
cargo run -- resource guard APPID \
  --data-root /path/to/macroton-data \
  --runtime-root /path/to/runtime-registry \
  --max-rss-mib 1600
```

The guard includes known Wine/Steam processes and arbitrary Windows game `.exe`
processes only after an open path proves ownership by the exact AppID prefix.
Use `resource status APPID --json` to preserve per-PID and category attribution.
Use `resource status-all --data-root PATH --json` for a read-only aggregate over
every numeric AppID directory in one Macroton data root. It reports per-AppID
totals without exposing commands or prefix paths and fails if one PID is
ambiguously attributed to two prefixes. `over_limit` is diagnostic only; the
command never stops processes. Exact-prefix `resource guard` remains the
containment mechanism.
Crossing the ceiling triggers the verified prefix-scoped shutdown and leaves
native macOS Steam and other prefixes alone. At terminal states the guard log
must contain its peak category split; a total without attribution is weaker
evidence. Preserve every immutable record under
`games/APPID/resource-guard-evidence/`; its terminal `reason` distinguishes
`rss-limit`, `time-limit`, `prefix-idle`, and `startup-no-process`. After
shutdown, require a zero-process status—the command now fails closed if
`wineserver -k` and the strict helper cleanup leave a game `.exe`.

For an explicit Windows Steam memory A/B, compare otherwise identical profiles
using `steam_client_mode=standard` and `low-memory-experimental`. Only compare
records whose `evidence_scope` is `steam-profile-session`; preserve both
profile fingerprints, exact-prefix peak JSON, CEF command-line evidence and
the full lifecycle outcome. A lower RSS value is not an acceptance result when
the client exits before login or the game never becomes `running-observed`.
The experimental mode uses `-nofriendsui`, `-nochatui`, and `-cef-disable-gpu`;
do not
add unsupported historical switches merely because older web posts mention
them.

After both terminal records exist, run:

```bash
macroton resource compare APPID \
  --standard games/APPID/resource-guard-evidence/STANDARD.json \
  --low-memory games/APPID/resource-guard-evidence/LOW_MEMORY.json \
  --standard-profile compatibility/APPID.conf \
  --low-memory-profile compatibility/experiments/APPID-low-memory.conf \
  --phase game --min-seconds 300 \
  --output games/APPID/resource-comparison/PAIR.json
```

The comparison fails closed unless both records used schema 3, observed Steam
and the game, are session-attributed, differ only in the intended client mode,
have supplied profiles whose only normalized difference is that mode, and have
comparable duration and sampling cadence. The selected phase must remain
present for a continuous run of at least `--min-seconds`; a one-sample game
appearance followed by five minutes of Steam alone is rejected. Use `--phase client` for a
login/idle-client comparison; it does not satisfy the gameplay memory gate.

For owner sign-in, prefer a profile with `steam_client_mode=standard`. The
low-memory and CEF modes are retained for game-launch experiments and
diagnostics; their compositor flags can produce a black native Steam surface
on some Wine/macOS WindowServer combinations even when the client is alive.

Prefer `steam login APPID --profile PATH` over using `launch` to provoke a
first-login UI. It verifies bootstrap provenance, refuses locked or unknown
macOS console state, starts no game, and automatically starts a bounded
exact-prefix guard. A fresh Steam install may first report
`client-update-progress-observed; login-window-not-yet-created`; follow it with
`steam client-status APPID --json` and require fresh progress before waiting
for the login window. The status rate and ETA must come only from a monotonic
15–120 second window in the same updater attempt; counter resets and day
boundaries deliberately produce `unknown` rather than a misleading estimate.
`steam lifecycle-status` distinguishes active and
five-minute-stalled updates so automation cannot mistake a resident updater
for `login-ui-active`. Record `prefix-status` before and after: stored login
metadata is only a local checkpoint, while transport, entitlement and
Steamworks remain separate runtime gates. A locked-session rejection must leave
resource status at zero before and after.

Treat `client-wait` completion as scoped to the latest observed updater
attempt. A clean prefix may first download a bootstrap client, report current,
and only after the next `steam login` launch request a distinct win64 client
payload with a new total. That second attempt is not counter regression. Keep
the launched prefix alive and run `client-wait` again; do not capture a QR
until the localhost CEF login target actually exists.

Macroton's bounded client, login, install, Cloud and exit-evidence wait commands
must hold `PreventUserIdleSystemSleep` only while their process is live. A
bounded live check should observe `/usr/bin/caffeinate -i -w <macroton-pid>` and
then confirm that assertion disappears when the command exits; display sleep
and persistent power configuration remain unchanged.

If the post-update WindowServer login surface is still black, build the
same-prefix CEF diagnostic twice and require byte identity before using it:

```sh
scripts/test-build-cdp-probe.sh
scripts/build-cdp-probe.sh /private/tmp/macroton-cdp-probe.exe
scripts/find-steam-login-cdp-target.sh
scripts/capture-steam-login-qr.sh APPID /absolute/path/to/wine \
  "$HOME/Library/Application Support/Macroton" 300
macroton steam login-wait APPID --timeout-seconds 600
```

The CEF-debug Steam mode includes `-cef-disable-gpu` while retaining
`-cef-enable-debugging`. This is intentional: the localhost DevTools endpoint
and owner-assisted QR capture remain available, while the GPU-process fan-out
seen in the updated Win64 client is avoided. Record the resource guard peak for
the login attempt; do not raise the limit indefinitely or treat a live client
as authentication evidence.

When the login popup is created but remains black, use the per-game
`*-opaque-login.conf` experiment. It adds `-cef-force-opaque-backgrounds` to
the no-GPU login path; the expected diagnostic is `transparency is force
disabled` followed by a visible `700x440` login window in `webhelper.txt`.
This is a compositor mitigation only and still requires live authentication
and resource-guard evidence.

The probe may expose only the owner's short-lived QR page through Steam's
already-open localhost DevTools endpoint. Never persist its screenshot, and
still require `prefix-status` plus live transport evidence after the scan.
The capture helper asks the page via the same local DevTools target whether its
QR image is complete and has non-trivial dimensions before taking the PNG. This
avoids publishing the login page's transient loading spinner over an incomplete
QR code; the QR itself may rotate periodically after it is ready.
The optional final argument waits a bounded number of seconds for the unique
login target. Timeout distinguishes an unavailable CEF endpoint (the client may
still be updating or may not have opened login UI) from a reachable endpoint
without a unique sign-in page; neither case produces a QR file.
The bounded `login-wait` command combines those signals without exposing an
account identifier: stored login metadata, numeric userdata, an authenticated
connection context and a live primary client must all be present. It fails
closed if the client is still updating, exits, or the deadline expires.
The Rust `steamcmd login` and `steamcmd acquire` commands also fail closed when
stdin is not a real TTY; use the visible Terminal helper or its
`--print-command` mode for owner-entered credentials. A background agent must
never be allowed to sit waiting for a password prompt.
`steam install` repeats the live connection, authenticated-context and primary
client checks immediately before submitting the numeric Steam protocol request;
an old stored-login file alone is never sufficient acquisition evidence.
The client-only login plan intentionally does not require an installed game;
otherwise first authentication could never acquire its owned depots. BuildID,
depot-manifest and PE-entrypoint gates remain mandatory for game `plan` and
`launch`.

Use `prefix-status APPID --json` and `launch-status APPID --json` for automated
preflight and lifecycle polling. Both omit raw account-local paths and log
lines. A stored login is not live authentication, and a Steam
`running-observed` event is not current host liveness unless its PID is also
present in `resource status APPID --json` for the exact prefix.

Before an interactive test, expose the same gate in machine-readable form:

```bash
cargo run -- steam session-status --json
```

Proceed only when `console_state` is `unlocked` and
`interactive_steam_allowed` is `true`. A locked/unknown result is an expected
preflight block, not evidence that Steam or the target game failed. The JSON
also includes an `action` field (`unlock-macos-desktop` while locked) so a GUI
or automation wrapper can present the next safe operation without guessing.

Capture a machine-verifiable checkpoint without overwriting prior evidence:

```bash
cargo run -- evidence snapshot APPID --profile compatibility/APPID.conf \
  --data-root /path/to/macroton-data \
  --runtime-root /path/to/macroton-runtimes \
  --output /absolute/path/to/new-snapshot.json
```

The snapshot is profile-fingerprinted and redacted. `profile_runtime_match`
proves that the exact AppID prefix bootstrap names the selected runtime ID and
installed runtime fingerprint; a missing runtime is recorded as `null`/`false`,
not silently accepted. `profile_session_match` additionally binds an active
Steam/game session to the exact profile, client mode and renderer; an idle or
stopped prefix correctly reports false and a null `steam_session_sha256`. Its
`not_inferred` list is part of the contract: do not
remove a gate merely because lifecycle, RSS or Cloud cache agreement happens
to be green.

Once focus, motion proxy, interaction, PCM presence, runtime independence and
normal-exit evidence have each passed, run `evidence bundle` with all six
create-new JSON files. The bundler requires identical AppID, profile, runtime,
Steam-session hash, game PID, client mode, renderer and graphics API across the
live-session artifacts. It separately requires a `complete` fresh Cloud
roundtrip for the selected profile, current Steam exit code 0 and zero
exact-prefix residuals. `restored` is recovery from quarantine and must fail
this acceptance gate. Preserve the resulting source hashes. The bundle still
does not prove human visual/audio quality, semantic controls, gameplay
correctness or in-game save reload.

Use `evidence bundle-status` against a canonical evidence directory at any
time. It is read-only and returns all six artifact states plus runtime,
cross-session consistency, Cloud, exit and residual-process gates in one JSON
object. This is the preferred resume/checklist interface; do not infer a pass
from one historical green field when `automated_bundle_ready` is false.
Schema 2 exposes `runtime_state`, `blocking_gate` and the first ordered
`next_action`: runtime provenance precedes live artifacts, followed by
single-session consistency, fresh Cloud, normal exit and zero residuals. This
also distinguishes a wrong runtime root from a missing capture instead of
collapsing both into `runtime_ready=false`.
Missing or invalid live artifacts produce a whole-session capture/recapture
action because the six artifacts must share one Steam session hash and game
PID. `blocking_artifact` identifies the first bad file without encouraging a
cross-session repair.

Use `evidence matrix-status` to apply the same ordered gates to every formal
`APPID.conf` directly under the compatibility directory:

```sh
macroton evidence matrix-status \
  --profiles-dir /absolute/path/to/compatibility \
  --evidence-root /absolute/path/to/live-evidence \
  --failure-dir /absolute/path/to/failure-records \
  --failure-source 108710=/absolute/path/to/108710-session-notes.md \
  --data-root /path/to/default/data \
  --app-data-root 606680=/path/to/silver/clean-data \
  --runtime-root /path/to/managed-runtimes
```

Evidence is read from `<evidence-root>/<appid>/{focus,motion,interaction,audio,independence,exit}.json`.
Only regular direct-child profiles whose filename matches their AppID are
accepted. Nested experiments and examples are excluded; symlinks, duplicate
AppIDs, unreadable entries and unused data-root overrides fail closed. Matrix
readiness is the conjunction of each profile's ordinary bundle gates, not the
existence of a profile or historical launch.

Schema 2 separates the eventual `evidence_*` blocker from the immediately
executable `operational_*` action. Its ordering is runtime provenance, unlocked
interactive console, bootstrapped/authenticated Windows Steam, live evidence,
fresh Cloud, normal exit and cleanup. Thus a locked, signed-out prefix reports
`unlock-console` now instead of prematurely instructing capture.

Pass `--failure-dir` to include canonical `${appid}-failure-*.json` records.
Every AppID with matching records also requires exactly one absolute
`--failure-source APPID=PATH`; the command re-hashes that source and verifies
the AppID, current profile fingerprint, source hash, and unique failure gate.
An observed failure is reported as `acceptance_state=known-human-failure` even
while its next operational action remains a retest step. Missing, mutated,
duplicate, or unused bindings fail closed.

The automated bundle is not the final gameplay claim. Once the operator has
reviewed the exact captured session, use `evidence record-review` to record
`pass` or `fail` for visual correctness, audible quality, semantic input,
gameplay correctness, in-game save/reload and sustained playability. The
record is bound to the selected profile fingerprint and SHA-256 of the exact
automated bundle. `evidence acceptance-status` fails closed on any binding or
schema mismatch and returns `gameplay_acceptance_ready: true` only when every
human gate passes. A failure record is preserved as evidence; it is never
silently converted into a pending or successful result. Release licensing,
redistribution, other hardware and future runtime regressions remain separate.

When a title cannot reach the complete-bundle stage, use `evidence
record-failure` with one of the canonical gates and a lowercase stable failure
code. This creates a profile-bound human failure record containing the SHA-256
of the source session notes. `evidence failure-status` re-hashes those notes
and rejects profile, AppID, source, gate, code, schema or status drift. This is
the appropriate representation for an observed black frame, unacceptable 3D
performance, incorrect input semantics, failed save reload or absent normal
exit. It does not convert a human observation into automated telemetry.

To evaluate a launch-only Steam client lifetime, first prove
`launch-status=running-observed`, then run `steam trim-client APPID --profile
PATH`. The command must show that the same exact-prefix game PID survived while
Steam/CEF disappeared. It rejects an already-absent client, a non-game tracked
PID, or a profile/runtime/session mismatch. Preserve the immutable
`trim-client-evidence` JSON containing the source identities and quantified RSS
reduction. Continue observing gameplay, input and audio. After a
normal game exit, restart official Steam and wait for Cloud `synced`. Reject
this mode for the title if the game exits, Steamworks-dependent functionality
breaks, or post-exit Cloud reconciliation cannot be proven.

For an interactive handoff that should trim Steam only after the configured
post-launch game executable appears, use
`scripts/launch-with-trim-client.sh APPID PROFILE DATA_ROOT RUNTIME_ROOT`.
	The wrapper derives `executable=` from the profile, unless the profile
	explicitly declares `trim_after_executable=`; this prevents a Launcher
	process from being mistaken for the handoff point. It polls only the exact
AppID resource inventory, and invokes `steam trim-client` once that executable
is live. Its default limit is 2200 MiB (`MACROTON_TRIM_MAX_RSS_MIB` overrides
it); timeout and signal paths stop only the requested AppID and preserve its
session marker.

Portal is a recorded negative case for this optimization: its window
disappeared after `trim-client` removed Steam. Do not enable launch-only client
trimming for AppID 400 unless a new session proves continued presentation,
input, audio and post-exit Cloud reconciliation.

`steam launch-status` reports the host process as `process_id` and preserves
Steam's raw Windows PID separately as `steam_process_id`. Evidence collectors
must bind macOS window/process inspection to `process_id`; treating the Windows
PID as a host PID can select an unrelated process or lose the live game.

Use `steam cloud-wait APPID --profile PATH` for the bounded reconciliation
gate. It is intentionally available only when the profile declares
`cloud_files`; each declared local file must match the cache and decoded remote
copy by content. If Steam has removed an uncompressed staging copy, a synced
cache entry whose size and SHA-1 directly match the local file is sufficient;
the report labels that path `synced-cache direct`. A timeout, missing local
file, stale hash, compressed object without a decodable remote copy, or profile
without mappings is not a Cloud pass.

The gate also requires zero client-side pending launch operations. A matching
cache with `Operation '3'` (or another pending operation) remains a failure
until Steam resolves the session; this prevents Cloud status from being
reported as launch-ready when Steam still holds the game at
`pendingcloudsessions`.

`steam install APPID` now warms the authenticated Steam single-instance client
and waits for its CEF readiness marker before dispatching `steam://install`.
This avoids the previously observed race where an install URI sent during
client bootstrap was silently dropped and `install-status` remained
`not-requested`.
The readiness probe accepts either a fresh `webhelper.txt` UI marker or the
client's fresh `console_log.txt` `System startup time:` marker; the latter is
needed by Wine Steam builds whose webhelper log has no stable UI marker.

Cloud mappings use `REMOTE_PATH|LOCAL_PATH`. An unqualified local path is
backward-compatible shorthand for `user:LOCAL_PATH`; `steam-install:LOCAL_PATH`
resolves below `steamapps/common/<steam_install_dir>` and is rejected at profile
parse time when `steam_install_dir` is absent. Keep the root explicit for new
profiles so install-root saves are never silently checked under the Wine user.
`steam-remote:LOCAL_PATH` models API-backed Steam Remote Storage files under
the one selected account's AppID cache. These files move with the cache during
roundtrip quarantine and restore; do not also treat them as user/install files.
Compressed remote objects remain fail-closed unless decoded content can be
authenticated.

`steam cloud-status` also reports the latest client-side `pending remote
operations` block from `cloud_log.txt`. This is deliberately separate from
the cache's `synced` state: Steam may have matching local/remote bytes while
still refusing an AppID launch until that pending launch operation is resolved.
That output is diagnostic evidence of a Steam client/session blocker, not
evidence that the game should be started through a direct executable.
Pass `--json` when a launcher needs machine-readable `state`,
`launch_ready`, `pending_remote_operations`, and per-file comparison states.

When a CEF-debug profile is explicitly authorized, use the port recorded in
the current `webhelper.txt` command line (the current Macroton profiles use
8080). The helper can extract it directly from the isolated prefix log:
`node scripts/resolve-steam-cloud-dialog.js --webhelper-log "$MACROTON_DATA/games/26800/prefix/drive_c/Program Files (x86)/Steam/logs/webhelper.txt"`.
The helper is read-only and prints candidate continuation buttons. Only pass
`--apply` after visually confirming the isolated Braid dialog. The script
accepts only `Play Anyway`, `Continue`, `Launch Anyway`, or `Start Anyway` and
never clicks login or store controls.
The helper rejects non-numeric or out-of-range ports before making any request,
always constructs a `127.0.0.1` endpoint, and rejects unknown command-line
options instead of silently ignoring them.
Run `scripts/test-steam-cef-port.sh` to verify latest-port selection and the
non-local endpoint refusal without starting Steam.
The mutating form additionally requires
`MACROTON_ALLOW_SESSION_DEBUGGING=1` alongside `--apply`.

For an owner-assisted recovery session, `scripts/resolve-macroton-cloud.sh`
combines the isolated Steam login and the fail-closed `cloud-wait` gate. It
keeps waiting while the user resolves the visible Windows Steam confirmation,
then exits only after the declared files are content-verified and
`launch_ready=true`:

```sh
scripts/resolve-macroton-cloud.sh APPID PROFILE "$MACROTON_DATA" \
  "$MACROTON_DATA/runtimes" 900
```
The script never selects local/remote content and never touches another AppID.
Set `MACROTON_CLOUD_MAX_RSS_MIB` to a lower positive value on memory-limited
hosts; the default is 3500 MiB.

A title whose official Steam feature list does not advertise Steam
Cloud must declare `steam_cloud=unsupported`, `steam_cloud_source` pointing to
that AppID's official Store page, and `steam_cloud_reviewed_date`; it must omit
`cloud_files`. Automated
evidence then records policy `unsupported`, transaction ID `0`, and phase
`not-applicable-unsupported`; it still requires in-game local save/reload in
the human review. Leaving both policy and mappings absent remains unresolved
and cannot produce a passing bundle.

The verifier requires an i686 MinGW compiler, confirms both registry views,
executes an intentional access violation, and passes only when the fixture
exits nonzero with zero `winedbg` processes. Do not run it while an unrelated
Wine debugging session is active.

For a real game process, a successful standalone probe is insufficient. Prove
that the game mapped the exact verified DXVK and MoltenVK artifacts before
recording a Metal FPS result:

```sh
scripts/verify-dxvk-d3d9-game-process.sh \
  GAME_PID \
  /path/to/private-game-view/d3d9.dll \
  /path/to/runtime/lib/wine/x86_64-unix/libMoltenVK.dylib
```

The verifier uses process mappings and SHA-256 identity. It fails if D3D9 fell
back to the prefix's Wine builtin DLL, if MoltenVK is absent, or if either
mapped file differs from the expected artifact.

For a MoltenVK `.gputrace`, also require live provenance before stopping the
game:

```sh
macroton evidence verify-gpu-capture APPID --profile PATH \
  --capture /absolute/path/frame.gputrace \
  --data-root /path/to/data --runtime-root /path/to/runtimes \
  --output /absolute/path/frame-evidence.json
```

The verifier accepts a regular capture file or a symlink-free directory tree,
requires every member to be nonempty, hashes sorted relative names/sizes/file
digests, and requires the tree to remain identical across the bounded stability
interval. Its pass proves only that the live selected runtime produced stable
GPU command-capture bytes; black presentation, rendering correctness, FPS,
input and playability remain separate gates.

The small offscreen workload can be run against both backends with:

```sh
scripts/benchmark-d3d10-runtime.sh /path/to/plain-wine/bin/wine wined3d
scripts/benchmark-d3d10-runtime.sh /path/to/wine-dxmt/bin/wine dxmt
```

Compare results only when both sides complete under the same desktop lock,
power, thermal, resolution, and runtime conditions. Never turn this synthetic
number into a game FPS claim.

Portal (AppID `400`) is a second-engine reference for PE32 Source/D3D9. Its
Steam-managed multi-depot run reached an input-responsive main menu, but not
gameplay, and is recorded in
[`docs/evidence/400-main-menu-2026-08-12.md`](evidence/400-main-menu-2026-08-12.md).

Braid (AppID `26800`) is the custom-engine/SDL2/D3D9 Effects reference. It
requires native `d3dx9_43` and `d3dcompiler_43` as one component and must use
WineD3D's OpenGL renderer on the current Apple Silicon runtime. Its jigsaw
screen is a color-sensitive regression gate: the hub alone does not detect the
Vulkan/MoltenVK red-tint failure. Keep the game's default VSync enabled because
`-no_vsync` worsens flicker, but do not mark the temporal gate passed: the user
still observed focused-window flicker with default VSync. A focus-aware
recording or direct user observation is required because an unfocused capture
and still image both produced a false negative. The
current gameplay/rendering record is
[`docs/evidence/26800-gameplay-rendering-2026-08-12.md`](evidence/26800-gameplay-rendering-2026-08-12.md).
The unpromoted `compatibility/experiments/26800-opengl-serialized.conf` profile
is the next single-variable candidate; do not copy it over the accepted profile
until color, focus, input, audio, exit, and Cloud gates all pass after a clean
restart.

Steam Cloud synchronization is performed by the authenticated Windows Steam
client inside the Wine prefix. The native macOS Steam UI is not authoritative
for that client's current sync status. Inspect the managed prefix directly:

```sh
cargo run -- steam cloud-status 606680 \
  --profile compatibility/606680.conf \
  --data-root /path/to/macroton-data
```

A Steam log completion is insufficient when the profile declares save files.
The command compares cache, staged remote copy, and actual Wine-user content by
size and SHA-1. Steam may stage an AutoCloud object as a four-byte uncompressed
length followed by a zlib stream; Macroton verifies the cache against those
physical bytes, then verifies the decoded content against the game file. The
comparison reports `encoding steam-zlib` when this path is used. Only
`State: synced` with per-file `match` evidence satisfies the upload side of the
Cloud gate. `local-drift` remains an explicit failure even if the Steam cache
itself has `syncstate=1`.

For the stronger fresh roundtrip gate, begin from a stopped, content-verified
prefix with no client-side pending launch operations and run
`steam cloud-roundtrip begin`. After an in-game save mutation,
normal exit and upload, stop the prefix and run `confirm-upload`, followed by
`quarantine`. The latter moves—not deletes—both declared local files and the
AppID's local Steam Cloud cache. Restart the authenticated Windows Steam client
to force cache reconstruction/download, stop it, and run `verify-download`.
Success requires post-checkpoint upload and download events plus identical
SHA-1 content. Use `status` after interruption and `restore` before retrying if
the download did not recreate data; restoration refuses all overwrites.

When a profile has changed since an obsolete baseline transaction was created,
`restore` may reject the new profile fingerprint. For a baseline-only
transaction that is no longer needed, use the explicit recovery command:
`steam cloud-roundtrip abandon APPID --profile PATH --data-root PATH --confirm`.
It requires an idle prefix and moves, rather than deletes, the active marker to
`transaction.abandoned`; the original transaction remains available for audit.

## Release-only gates

Before rating Silver playable, repeat on a clean Apple Silicon Mac with no
commercial Wine distribution installed and no host-specific DNS override. Then
verify gameplay start, audible output, save/load, controller or keyboard input,
and a long session. Main-menu compatibility alone should be labeled exactly as
main-menu compatibility.
# AoE3 crash evidence

For repeatable Logo-stage diagnostics, use the AppID-bound collector:

```sh
scripts/capture-aoe3-crash-evidence.sh \
  compatibility/experiments/105450-shadowalias-vulkan.conf \
  /private/tmp/macroton-aoe3-evidence-$(date +%s)
```

The optional third argument sets the post-submit lifecycle wait in seconds;
the default is 30 (override with `MACROTON_AOE3_WAIT_SECONDS`). If Steam never
reports `running-observed`, the collector records that as a launch-path
failure instead of claiming a game crash.

The collector records the pre-launch `gameprocess_log` line boundary, waits
for the tracked lifecycle, and writes only the new game-process lines alongside
the Steam lifecycle, Wine exception and D3D9 logs. It stops only the exact
105450 Macroton prefix on exit. It is diagnostic only; a Logo observation is
not a playable-game pass.
