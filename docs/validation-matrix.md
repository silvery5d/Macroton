# Macroton validation matrix

> Historical record: this matrix contains earlier Braid and 3D diagnostic
> runs. The current product scope is 2D-only; these entries are not current
> Macroton support targets.

The matrix is inventory-first and eligibility-first. A game with a currently
working official Apple Silicon/macOS build is excluded even when its Windows
build is useful to Wine developers.

## Current 2D inventory snapshot — 2026-09-12

The authoritative inventory scan (`steam inventory --2d-only --with-appinfo`)
currently identifies three in-scope candidates: Silver (606680), Heroes of
Might & Magic III HD Edition (297000), and CHRONO TRIGGER (613830). Silver is
the strongest end-to-end candidate and has current Cloud consistency evidence.
Heroes has a visually verified Windows launcher but no trustworthy Play-input
or gameplay pass yet. CHRONO TRIGGER is Windows-only in the current local
inventory and remains an acquisition/runtime candidate; its stale exact-prefix
guard was stopped before this snapshot. No 3D title is accepted by this
matrix.

The next acceptance order is: (1) complete a foreground-safe Heroes launcher
click and visible 2D game scene; (2) perform Silver save/load and Cloud
roundtrip evidence; (3) acquire and validate CHRONO TRIGGER only after its
Windows depot is proven. Process-only launch, cached metadata, or a screenshot
captured while another macOS app is frontmost does not count as gameplay.

## Current acquisition boundary — 2026-08-26

Braid (AppID `26800`) remains the next eligible inventory target. The current
Macroton data root contains the independent `/games/26800/prefix` and its
installed Windows Steam tree. The profile-pinned Windows depot is installed
and the owner-assisted Wine Steam session is stored. A fresh formal launch on
2026-08-26 produced a focused, full-color Braid campaign hub and a live game
process after the authenticated client recovered from one transient
`LogonFailure No Connection`; this is a rendering smoke pass, not full
playability.

The later retry after a forced-stop/Cloud cycle is currently blocked by Steam's
own `SynchronizingCloud "pendingcloudsessions"` state even though Macroton's
local/remote content verifier reports `synced`. It produced no game process and
is recorded as a negative launch observation, not a gameplay pass. A reversible
CEF-helper experiment was rolled back after the preserved helper reached a
2.1 GiB RSS peak and was incorrectly counted as a game; the resource classifier
now treats preserved Steam helpers as `steam-cef`.

A further reversible diagnostic moved only `userdata/<steamid>/26800` to a
timestamped quarantine and retried launch; Steam recreated no game process and
still emitted `pendingcloudsessions`. The directory was restored and the Cloud
verifier again reports the original save as `synced`, so no save or remote data
was lost.

The Cloud status command now preserves and prints the separate client-side
`Operation '3' from machine ''` record found in `cloud_log.txt`; this explains
why a cache-level `synced` result does not make the Braid launch ready.
The `cloud-wait` gate now fails closed while that operation remains present,
even when all declared bytes match.
The temporary `compatibility/experiments/26800-cloud-debug.conf` profile is
diagnostic-only; it is not part of the supported runtime or release profile.

The acquisition route is now explicitly Wine Steam-only. SteamCMD login is not
part of the user workflow after repeated server-side `Invalid Password`
responses despite a verified local account name and stable hidden-input test.
On 2026-08-26 Braid's independent prefix was bootstrapped, Wine Steam updated
successfully, QR login completed, and `login-wait` observed stored login,
connected transport, authenticated account context, and a live client. The
install UI was exposed through the local CEF diagnostic surface and the
profile-pinned Windows depot was downloaded. The next gates are real
input/audio/flicker validation, clean-prefix Cloud reload, and sustained
normal exit.

## Latest Heroes III continuation evidence — 2026-08-28

The most recent manual `Play` click was also captured. The launcher was
visible, but the managed prefix never produced a new `HOMM3 2.0.exe` process;
the resource guard later reported the prefix idle and stopped it after the
five-minute handoff timeout. `steam launch-status` therefore reports a stale
historical `running-observed` event, while `lifecycle-status` correctly marks
the run as `stale-running-event` with no live game PID. This is not counted as
successful gameplay evidence.

Heroes III (AppID `297000`) remains an eligible Windows-only target. The
reproducible DXVK profile now has a low-memory Steam variant at
`compatibility/experiments/297000-dxvk-low-memory.conf`; Macroton launched it
without CrossOver and exposed the real PE32 Launcher in a separate prefix.
The low-memory client flags were confirmed in the Steam command line, but the
Wine Steam CEF stack still reached roughly 2.2 GiB RSS before the Launcher
handoff. A 2.0 GiB guard therefore stopped the prefix before Play could be
validated; a one-off 2.6 GiB diagnostic run kept the Launcher alive.

The current result is still `gameplay blocked`: no new `HOMM3 2.0.exe` process
was observed in the controlled attempt, and the Launcher was not foregrounded
for a valid Play click. The profile's `SDL_RENDER_DRIVER=opengl` setting is
retained for the next foreground-click run. Do not promote this as a gameplay
pass until a visible game window, input action, renderer result, audio, save
reload, Cloud state, and clean exit are all captured.

The follow-up private Launcher-bypass experiment reaches the Steam-owned
`-applaunch` path while the fingerprinted payload view substitutes the
verified PE32 game entrypoint for `HOMM3Launcher.exe`. A visible `Assert
Failed` window seen during these runs was later matched to Steam's own
`CWorkThreadPool::~CWorkThreadPool` assertion and dump, not to the game; it
therefore does not prove an SDL/D3D9 render-target failure. The bypass remains
diagnostic-only and preserves the original Launcher under the private source
tree.

The launch-preparation path now also treats DXVK renderer transforms as
source-preserving transactions: an installed prefix tree is preserved before
the private view is created, and a renderer DLL symlink is detached before a
writable copy is staged. This prevents future Heroes III-style experiments
from modifying the authenticated Steam payload. Formatting and the full Rust
and CLI regression suites pass after this change.

The renderer isolation regression suite now includes an explicit symlink case:
the source `d3d9.dll` remains byte-for-byte unchanged while the private view
receives a regular writable copy. The current test totals are 187 Rust tests
and 58 CLI tests, all passing.

Preparation now also recovers a uniquely identified interrupted
`.macroton-source-*` transaction when a prior transform failed before view
commit; this keeps a failed experiment from leaving the next install-status or
launch path in `manifest-only` state.

The source-preservation changes pass `cargo fmt --check`, Clippy with
`-D warnings`, 188 Rust tests, and 58 CLI tests. The suite includes an
idempotence test for recovering an interrupted hidden install tree.

Mass Effect (AppID `17460`) was retried from its stored-login prefix through
the official Wine Steam install URI. Steam completed client startup and
reconnected, but created no `appmanifest_17460.acf`, depot transaction, or
payload. A diagnostic attempt to submit a second URI after startup was
reverted after it spawned duplicate CEF helpers and reached 2.86 GiB RSS;
Macroton stopped the exact prefix and verified no residual processes. This is
an acquisition/Steam handoff blocker, not evidence of game incompatibility.

A follow-up controlled attempt first established a single persistent-login
Steam client (`persistent-login-client-live`, CM logon `OK`) and then sent the
install URI without launching a second client. Steam still showed its updater
surface and wrote no AppID manifest or depot state. The attempt was stopped
and its detached resource guard was terminated explicitly; no credentials were
read and no payload was acquired. Further automated URI variants are paused
until the user confirms the install action in the visible Windows Steam UI.

## Latest continuation evidence — 2026-08-25

- Steam lifecycle robustness: a new `-applaunch` console event now resets the
  previous tracked PID instead of preserving a dead PID indefinitely. This
  prevents stale `running-observed` records from blocking the next launch as
  `stale-running-event`; the parser regression suite and Clippy pass.

- Steam Cloud lifecycle: `SynchronizingCloud "pendingcloudsessions"` is now
  classified as `waiting-cloud` only while the exact-prefix Steam client is
  live, and as `stale-waiting-cloud` after the client exits. This keeps a
  historical console event from being mistaken for an active game blocker.

- Silver (606680): Macroton retains the generated `SAVEGAME -> .` directory
  alias in the accepted runtime. A reversible A/B with a real `SAVEGAME`
  directory (both file links and physical copies) caused Silver to exit during
  initialization, so neither layout is promoted. Load-menu and clean-prefix
  Cloud roundtrip evidence remain pending.
- Silver (606680): game-launch Steam arguments no longer include `-silent`.
  A clean build produced a visible Steam window and a real `silver.exe` window
  without the prior hidden-client gate. This is a bootstrap/visibility result,
  not yet a save-reload pass.

- Silver (606680): background music is confirmed in the accepted
  WineD3D/OpenGL path. The managed prefix contains twelve `.sav` files plus
  `silver.opt` at the documented `%APPDATA%` location and they match the
  authenticated Steam cache, but direct Load-menu recognition is still
  unproven. A real Silver main menu was visible; host-side input injection did
  not yet open Load, so no save-reload or fresh Cloud roundtrip claim is
  promoted.
- Silver (606680): manual play confirmation closes the prior audio concern;
  background music is present and no gameplay/input issue was found. The only
  confirmed presentation defect remains the grayscale intro/transition video.
- Silver (606680): a live read-only `steam cloud-wait` on 2026-08-27 passed
  the stricter gate: 13 declared files matched and zero client-side pending
  launch operations were observed. This is current Cloud consistency evidence,
  not a fresh upload/download roundtrip.
- Silver (606680): a reversible `yuv444p` Theora intro substitution also
  remained grayscale under the real Steam/Wine path. The original intro was
  restored byte-for-byte; the remaining defect is in video presentation rather
  than source chroma subsampling.
- Silver (606680): the unlocked 2026-08-26 retry authenticated Steam as
  `silvery5d`, created a real `silver.exe` process, and cleaned it to zero
  residual processes. The resource guard recorded a 4,337.3 MiB Steam/CEF
  peak (Silver itself 259.7 MiB). Window compositing and input injection still
  prevented a trustworthy Load-list capture; no save mapping was changed.
- Silver (606680): after the lifecycle reset fix, a real pause-menu `Load`
  action changed the session from the newly-started cave scene to a different
  bridge/city scene with the player and companion. This is strong evidence of
  save loading through the accepted `SAVEGAME -> .` mapping, but the slot list
  itself was not captured, so full slot-by-slot/Cloud roundtrip acceptance is
  still pending.
- Silver (606680): `steamsysinfo.exe` and Steam error-reporting helpers are
  explicitly classified as `wine-helper`; they cannot create a false
  `windows-game` PID or gameplay pass.

- Silver (606680): a fresh standard-client regression exited with Windows
  status `0` but its CEF helpers peaked at about `2.82 GiB`, exceeding the
  configured `2.2 GiB` guard. A clean low-memory rerun verified that
  `-cef-disable-gpu -cef-force-opaque-backgrounds` reached the newly spawned
  Steam client, yet CEF still peaked at about `2.83 GiB`; the game itself was
  not the memory consumer. This is now recorded as a Steam CEF resource gate,
  not a game crash or gameplay pass.

- Braid (26800): a fresh low-memory Steam/Wine launch created the tracked
  PE32 game process and remained live during the observation window. The
  WindowServer probe exposed only untitled Wine surfaces, so no visible game
  frame or interaction was promoted; the instance was stopped cleanly. This
  is a launch-chain pass with visible-rendering evidence still pending.

- Braid (26800): the opaque-software Steam comparison did create a tracked
  `Braid.exe` process, but Steam CEF peaked at about `3.95 GiB` and the
  resource guard contained the prefix before a visible frame could be
  collected. This renderer/client combination is rejected for resource use;
  it is not a gameplay or rendering pass.

- Braid (26800): after the independent prefix login was verified, a fresh
  Steam launch reached the tracked PE32 entrypoint but exited with Windows
  status `0xC0000005` before a visible frame. The same run opened a new Steam
  sign-in surface during client startup, so login-session reuse is not yet
  stable. This is an authoritative startup-crash result, but it does not yet
  identify the failing game module or prove an in-game rendering failure.

- Silver (606680): the 2026-08-25 bounded Steam submission returned a normal
  AppID exit before a fresh tracked `silver.exe` PID was available. The exact
  prefix was cleaned to zero processes/zero RSS; see
  `docs/evidence/606680-steam-session-20260825.md`. This is not a new gameplay,
  input or audio pass.

- Silver (606680): after the exact-executable guard fix, a 54.86-second retry
  observed Steam but zero matching `silver.exe` samples; the peak was 2,082,864
  KiB, mostly CEF. The prefix again cleaned to zero processes/zero RSS. This
  closes the helper false-positive without promoting a game launch.

- Silver (606680): the real installed Windows depot and PE32 payload remain
  present, but the latest standard-profile Steam submission did not produce a
  sustained `silver.exe` process. The result is recorded as
  `submitted-no-game`, not as a gameplay pass; see
  `docs/evidence/606680-submitted-no-game-20260815.md`.
- Dishonored (205100): the isolated prefix now has a stored login and a live
  authenticated-client observation was recovered with an explicit 5 GiB
  install/login guard, but Steam did not create an install request or payload;
  the prefix was stopped afterward. It remains acquisition-blocked, not a
  gameplay result; see `docs/evidence/205100-bootstrap-20260815.md`.

## Evidence dimensions

Each candidate advances independently through these dimensions:

| Dimension | Pass evidence |
| --- | --- |
| Eligibility | Owned; official Mac path absent or unusable on the target Mac |
| Win32 | Real game entrypoint is PE32/i386, not only a 32-bit launcher |
| Bootstrap | Steam entitlement, depot install scripts, and required runtimes complete |
| Presentation | Correct scene-specific colors/effects, not merely a window |
| Input | State transition caused by keyboard, pointer, or controller input |
| Audio | Audible output plus working in-game volume/mute behavior |
| Save/load | A changed save survives normal exit and reload |
| Cloud | Upload, clean-prefix download, and reload; cache ends `synced` |
| Stability | Sustained gameplay and normal exit without a hung Wine server |

## Current executable/API evidence

The historical authenticated Windows tree was rescanned with PE architecture
recorded per observed executable. Architecture promotion is restricted to an
exact normalized path from Steam launch metadata or `config/checkguid(s)`; an
x86 redistributable beside an x64 game cannot create a Win32 pass. `steam
inventory --candidates-only` now separates installed PE32 entrypoint proof from
metadata-only candidates:

| AppID | Game | Real entrypoint | Static families | Evidence tier |
| ---: | --- | --- | --- | --- |
| 606680 | Silver | PE32/i386 `silver.exe` | OpenGL, SDL input/audio, WinMM, VC++ | `candidate-verified-win32-installed` |
| 221380 | Age of Empires II (2013) | PE32/i386 `AoK HD.exe` | D3D9, DirectSound, WinMM, legacy DirectX | `candidate-verified-win32-installed` |
| 26800 | Braid | PE32/i386 `braid.exe` | D3D9, SDL/DirectInput, SDL/DirectSound/WinMM | `candidate-verified-win32-installed` |
| 297000 | Heroes of Might & Magic III - HD Edition | acquired, gameplay blocked | Windows-only Steam build is installed in an isolated Macroton prefix; Steam launch and PE32 game process are proven, but SDL render-target initialization asserts under current Wine/DXVK | metadata candidate; acquisition and launch evidence recorded |
| 17460 | Mass Effect (2007) | historical PE32/i386 `MassEffect.exe` (current 17460 prefix has no payload) | D3D9, XInput/DirectInput, OpenAL/DirectSound, PhysX | historical `candidate-verified-win32-installed` |
| 108710 | Alan Wake | PE32/i386 `AlanWake.exe` | D3D9, XInput, FMOD/WinMM | `candidate-verified-win32-installed` |
| 400 | Portal | PE32/i386 `hl2.exe` | Source loads key graphics/audio modules dynamically; static imports are incomplete | `candidate-verified-win32-installed`; official-review result `eligible-verified-win32-legacy-mac` |
| 205100 | Dishonored | download incomplete | cached Win32 path and UE3/D3D9 preflight only | metadata candidate |

Static families select the tests; they do not pass runtime input or audio.
The next interactive order is Silver clean-prefix lifecycle, Age II black-frame
diagnosis, then Portal clean multi-depot replay. Mass Effect and Braid provide
the next distinct input/audio/prerequisite combinations. Dishonored waits for
the official payload rather than consuming desktop Steam memory on an
uncommitted download.

## Owned candidates and order

Ownership here is derived from the locally authenticated Windows Steam
client's per-account app cache, not from a public wishlist or store search.

| Order | AppID | Game | Coverage | Eligibility status | Next gate |
| ---: | ---: | --- | --- | --- | --- |
| 1 | 26800 | Braid | custom engine, SDL2, D3D9 Effects, keyboard, save, Cloud; color-correct focused OpenGL A/B complete; CSMT=3 rejected; compressed Steam Cloud object decoded and matched local save. The new bounded `cloud-wait` gate passes that one declared save against the historical shared root and correctly returns `no-evidence` for the independent AppID prefix. The independent audited prefix remains free of copied login state and currently idle | eligible: the current official Steam page explicitly marks the original release incompatible with macOS 10.15 or above; a tracked, content-addressed review resolves the stale dual-platform cache, and the installed Windows entrypoint is verified PE32 | sign in to the independent prefix, then continue focused flicker, clean-prefix Cloud reload, audio, controller and stability validation |
| 2 | 17460 | Mass Effect (2007) | Unreal Engine 3-era D3D9, EA launcher/config, large 3D payload; the historical real PE32 entrypoint and two-depot build are pinned, while the current clean public-WineHQ AppID 17460 prefix has no copied login or payload and zero residual processes | eligible: the current official Steam page lists Windows-only system requirements; a tracked, content-addressed review resolves incomplete local AppInfo, and the historical game entrypoint is verified PE32 | authenticate the clean prefix, accept the official first-launch EULA, acquire the pinned depots, then test launcher-to-gameplay and local save/reload; the official feature list does not advertise Steam Cloud, so the profile records Cloud as unsupported rather than inventing a roundtrip |
| 3 | 105450 | Age of Empires III (2007) | legacy D3D9 strategy game; local authenticated appinfo records Windows-only launch paths `bin\\age3.exe`/`age3x.exe`/`age3y.exe`, BuildID `44725`, and the pinned base/English depots 105451/105452; the reproducible profile pins `steam_client_mode=opaque-software-experimental` to avoid Wine CEF transparent-surface black windows; `mfc42` is installed for the PID generator dependency; `-windowed` was removed after isolated evidence showed it triggers an early access violation; warm Steam handoff now creates the game process reliably | eligible: inventory playtime/account record plus authenticated Windows Steam install in the isolated Macroton prefix | complete authenticated acquisition, verify PE32/D3D9/input/audio dependencies, then exercise the real strategy scene, mouse/keyboard input, save/reload, Cloud policy and sustained stability; warm handoff is proven, but the 2026-08-30 map-load run entered a 95–100% CPU Wine exception loop and is not gameplay evidence; DXVK/MoltenVK still exits earlier, so renderer/dependency remediation remains open |
| 3 | 108710 | Alan Wake | custom multithreaded engine, D3D9, XInput, Steam Remote Storage | eligible; PE32 and real WineD3D/OpenGL gameplay confirmed but active-scene performance is unacceptable. The patched SPIRV-Cross/MoltenVK 1.4.2 runtime fixes the comparison-sampler pipeline failure and visibly renders opening video/menu at roughly 115-187 FPS. Exact-PID Escape produced a main-menu transition. The authenticated save then entered a changing 3D sequence, but only at roughly 0.3-2 FPS. Neither a fake NVIDIA 8600 GT plus 2 GiB memory bound nor disabling DXVK `generalHazards` restored playable speed. The game rejects both 640x360 and 800x450 before surface creation, so those runs cannot test resolution scaling; 960x540 remains the lowest proven render size. During the slow scene the game was CPU-active (~127%, ~260 MiB RSS); reproducible sample parsing shows Rosetta JIT in 1,912/1,925 game-thread branches, while only 11/1,925 DXVK-submit branches touch MoltenVK. All three API-backed Cloud files now have explicit `steam-remote:` mappings and match the historical raw cache; the independent prefix still has no Cloud evidence. A full audit exposed 1,941 prefix-owned Wine service/helper residuals. A later wider arbitrary-`.exe` resource audit corrected the initial zero claim by finding two additional `SteamServiceTmp.exe` helpers (1,592 KiB); the explicit helper allowlist cleaned both with TERM and final schema-1 status is now zero. A private-view command-pressure attempt crossed the 1,600 MiB host ceiling at about 1,910.6 MiB before a scene capture, with Windows Steam/CEF dominating rather than the 30 MiB game; it yielded no HUD conclusion. The guard now attributes peak RSS by process/category and counts arbitrary game executables. Historical PNG intervals cannot be promoted to the new motion-rate proxy because no ordered high-rate timestamp/hash CSV was retained. The unacceptable 3D rate and absent normal-exit pass are now preserved as profile/source-bound `human-gate-failure` artifacts rather than prose-only conclusions. | keep accepted profile on WineD3D. Retain patched DXVK/MoltenVK only as an experiment; use a lower-overhead authenticated launch path, then preserve one reviewed moving 3D focus stream and run both focus and motion-rate gates before audio/save/stability promotion |
| 4a | 297000 | Heroes of Might & Magic III - HD Edition | turn-based strategy, Windows-only Steam build, multi-depot; authenticated independent prefix, manifest/payload and PE32 entrypoints are verified; Steam→Launcher→game process chain now works, but the game hits a known SDL `SDL_SetRenderTarget` assertion under Wine/DXVK and has not passed visible gameplay | eligible by official Windows-only store requirements; no native Mac build is advertised | isolate SDL/D3D9 render-target compatibility (800x600/windowed, renderer/runtime variants), then retest 2D rendering, pointer input, audio, save/reload, Cloud policy and stable exit |
| 4 | 205100 | Dishonored | Unreal Engine 3, D3D9, first-person raw mouse, XAudio, Steam Remote Storage; cached BuildID and launch path extracted into an experimental preflight profile. An official authenticated desktop-client install proved entitlement and selected six owned depots, but CDN retries never committed the payload and CEF helpers later crossed the 1,600 MiB guard. Valve SteamCMD completed self-update at about 253 MiB and refreshed anonymous AppInfo at about 305 MiB, confirming current BuildID `8335061`, Win32 launch path and base/English manifests. Macroton now has foreground login/acquire, status, exact manifest comparison, synthetic plan/launch bridging, PE input/audio/prerequisite inspection, an isolated AppID prefix and a 500 MiB distribution guard. Schema-2 acquisition evidence atomically preserves the accepted manifest and fails closed on profile, executable, runtime fingerprint, manifest hash, BuildID or full-depot drift. A separate clean public-WineHQ desktop prefix also now exists with no copied account/payload/Cloud state and zero residual processes. The first visible SteamCMD prompt remained unauthenticated and was stopped by the new 30-minute exact-prefix timeout with `0.0 MiB`; acquisition therefore remains correctly `missing`. Stored-login `steam://install/205100` reached a logged-on Steam transport but produced no manifest, content-log download record, or payload; the standard Wine Steam window was present yet completely black, and macOS Accessibility exposed no buttons or text, so there is no safe way to confirm installation without the CEF diagnostic path. The real aggregate gate now reports `blocked-acquisition` rather than failing on the pre-`steamapps` client layout. AppInfo has no UFS rule; the shared authenticated cache has 11 nonempty `syncstate=1` Remote Storage files whose raw content matches, plus 42 zero-byte pending placeholders, so its aggregate state remains `syncing`. The independent prefix declares the 11 content-backed files but correctly has no Cloud evidence. | eligible: current store lists Windows only; official Windows Steam accepted the install for the authenticated owner | complete a fresh interactive SteamCMD authentication and download; require acquisition `ready`, real PE32/D3D9/input/audio dependency evidence, then proceed to gameplay/input/audio/save/Cloud/stability without treating historical cache parity as a fresh roundtrip |
| 5 | 221380 | Age of Empires II (2013) | retired D3D9 strategy game; real 4.79 GB/seven-depot BuildID 3062235 install and PE32 `aok hd.exe` confirmed; legacy IE launcher crashes, while a Steam-tracked PE32 bridge reaches the game; the fail-closed private-view launcher substitution is now implemented and tested without mutating Steam's payload; Valve AppInfo officially declares the install-root UFS patterns, and the exploratory cache's two profile files match install-root content by size/SHA-1, but the exploratory prefix was not AppID-isolated and its last log state remains `syncing`; both experimental profiles now reserve a UFS-authorized named `.aoe2spgame` slot so the eventual Cloud gate mutates and restores a real in-game save rather than only settings; WineD3D exits before a window; patched DXVK/MoltenVK fixes the captured shadow-sampler compile failure and creates a low-CPU 1512x982 swapchain/window, but the foreground presentation remains pure black even with `SKIPINTRO`; an audited independent prefix and public client exist with zero copied account state, and exact stop now also catches Steam processes whose cwd is outside the prefix | eligible: Valve's current store API declares Windows only; authenticated account cache records 6,754 historical playtime minutes | account owner signs in to `games/221380/prefix`; then capture the first Metal frame, reproduce the staged bypass, fix black presentation, and continue match/input/audio/save plus isolated Cloud upload/download/reload |
| 6 | 400 | Portal | Source engine, multi-depot PE32/D3D9. Historical evidence includes a formal Steam launch with a rendered first 3D chamber, correct-looking neutral color, a focused temporal gate, a presentation-rate proxy, exit code 0 and a completed Cloud sample. That evidence came from an earlier disposable/content-cloned prefix and is not fresh-download proof. **Current state:** the persistent AppID 400 prefix is bootstrapped with stored login but has no verified game payload or path-bound PE32 entrypoint; `target-status` therefore returns `blocked-no-path-bound-win32-entrypoint`. | eligible by native viability review, but not currently runnable until authenticated acquisition proves the exact Windows payload | after the macOS console is unlocked, acquire the pinned manifests freshly into AppID 400; then verify PE32, fix the Wine window/Space presentation path, and capture same-session interaction, PCM/audio, save/reload, fresh Cloud quarantine/download roundtrip, watched sustained normal exit and human playability before promoting the experimental renderer |
| 7 | 606680 | Silver | SDL-era custom game, PE32 with static OpenGL/SDL evidence and observed Wine OpenGL/CoreAudio modules; official Mac build is legacy-ineligible. Formal Macroton Steam launch, visible cinematic/main menu, and one Options input-response transition are proven without CrossOver. A later strict PE/profile gate correctly rejected the stale `graphics_api=d3d9` declaration; the accepted profile now pins `renderer=wined3d-opengl` and `graphics_api=opengl`, because the default renderer presented the OGV intro in grayscale and emitted only sparse PCM. The profile also pins twelve save slots plus `silver.opt`; all thirteen historical local files match Steam's `syncstate=1` raw-cache size/SHA-1 through `cloud-wait`. A recoverable `cloud-roundtrip` transaction now requires post-baseline upload/download events and quarantines both saves and local cache, but correctly refuses to begin against the clean unauthenticated prefix's `no-evidence` state. Resource evidence is now append-only schema 3, records observed Steam/game processes, and has a strict profile-bound Standard/LowMemory comparator; the experimental profile's missing Cloud declarations were corrected so the parsed pair differs only in client mode. Fixture comparison passes, but no fixture value is promoted as live Silver memory evidence. A live-session-bound motion-presentation proxy now measures sampled change cadence/freezes without mislabeling them as engine FPS | eligible | unlock/login; collect matched Standard/LowMemory gameplay records; capture a reviewed moving 3D scene through focus stream plus motion-rate proxy; then exact-window/PID runtime audio plus listening pass, fresh Cloud transaction through in-game reload, controller if supported, long session and normal exit |

Heroes III received a fresh SDL experiment on 2026-09-13. Forcing `SDL_VIDEODRIVER=windows` with the software renderer did not change the outcome: only `HOMM3Launcher.exe` appeared, no `HOMM3 2.0.exe` process was observed, and the 1,400 MiB guard contained the prefix after Steam CEF reached about 3.98 GiB. The result is a reproducible negative launch/runtime observation, not gameplay evidence; the next experiment remains focused on SDL2/OpenGL-ES render-target initialization.

## Explicit exclusions

Portal state correction (2026-08-26): the authenticated AppID 400 evidence in
the row above came from a disposable/content-cloned prefix and is retained only
as historical evidence. The current persistent independent prefix is
bootstrapped with one stored-login record, but its game payload has not yet been
freshly acquired; no historical payload or Cloud state was copied into it.
`target-status` therefore remains `blocked-no-path-bound-win32-entrypoint` until
the unlocked desktop permits an authenticated install.
On 2026-08-27 Macroton submitted `steam://install/400` through that prefix;
Steam logged `ExecuteSteamURL`, but produced no `appmanifest_400.acf`, depot
content log, download progress, or payload within the bounded observation
window. The client reached about 1.7 GiB RSS without an install state change and
was stopped by the AppID-scoped stop path. This is an acquisition/Steam-client
handoff failure, not gameplay evidence and not permission to use a direct EXE.

Follow-up correction (2026-08-27): a user-visible Steam installation session
then completed BuildID `19017868` into the persistent AppID 400 prefix. The
formal Macroton Steam launch subsequently observed `hl2.exe` with the pinned
profile and a live Steam client; `lifecycle-status` reported
`running-observed`, and the resource guard measured 1.1--1.7 GiB RSS under the
2 GiB limit. WindowServer still reported the Portal window as off-screen during
the automated check, so this is real launch/resource evidence only. Human
visual, input, audio, save/reload, fresh Cloud roundtrip, and sustained-exit
gates remain open.

The follow-up six-second ScreenCaptureKit capture successfully focused the
Portal window and passed Macroton's `focused-temporal-sample` gate with 12/12
owner-matched frames, no black frames, and 12 unique samples. This upgrades
presentation evidence only; the remaining gameplay gates are unchanged. See
[`portal-2026-08-27-focus.md`](evidence/portal-2026-08-27-focus.md).

The same live window/PID then produced an eight-second 48 kHz stereo sample
with 423 valid buffers and an 82.5% non-silent ratio; Macroton returned
`pcm-presence-pass`. This is audio-presence evidence only and does not replace
content or human-listening review.

The same capture also passed the bounded motion-presentation proxy at 1.690
changed samples/second with a 685.7 ms maximum unchanged interval. This is not
an engine-FPS or playability claim.

A higher-rate 10.18-second stream then captured 300/300 focused frames and
passed the stricter proxy at 29.278 changed samples/second with a 71.3 ms
maximum unchanged interval. This remains a presentation-delivery proxy, not
an engine-FPS or human-playability result.

The formal `macos_interaction_probe` then sent one `Escape` event to the live
Portal PID and returned `input-associated-frame-change-pass` with a 0.505
changed-pixel ratio and stable foreground/PID ownership. This upgrades input
association evidence only; semantic action correctness and human playability
remain open.

- `Baba Is You`: official modern macOS build works; it is not a Macroton target.
- `The Witcher: Enhanced Edition`: current store supports Apple Silicon/macOS.
- `The Witcher 2`: current store lists macOS 11+ and Apple silicon support.
- `FEZ`: deferred until its current official Mac build is tested on the target
  host; do not count it merely because an old store requirement mentions macOS.

## Heroes III crash evidence — 2026-09-04

One Steam-owned WineD3D/OpenGL bypass run reached the tracked PE32 entrypoint
and exited with Windows status `0xC0000005` (access violation). This is an
authoritative crash code, but it does not by itself identify the failing module
or prove equivalence with a later user-triggered map-load crash. The isolated
control profile `compatibility/experiments/297000-bypass-wined3d-serialized.conf`
now pins WineD3D command-stream serialization for the next A/B run without
changing Steam ownership or payload files. Promotion still requires reproducing
the same level-load action and capturing the resulting module/renderer evidence.

The serialized run itself was inconclusive for gameplay: the Wine Steam client
terminated during startup and produced
`dumps/assert_steam.exe_20260904121742_1.dmp`. The dump identifies Steam's
`CWorkThreadPool::~CWorkThreadPool` assertion, while no Heroes III module or
map-load exception was recorded. This is a separate Steam-client/Wine
stability issue and must not be counted as a game crash or a serialized-
renderer pass.

The follow-up standard-client `forceANGLE` control reached the Steam-tracked
launcher replacement but exited after roughly five seconds with the same
Windows status `0xC0000005`; Steam recorded removal of PID 944 and no map-load
event. The `--forceANGLE` argument is therefore currently rejected as a
startup-unstable experiment, not a candidate fix for the user's later
level-load crash.

During the subsequent `-windowed` baseline, WindowServer inspection found the
Wine PID associated with the tracked Heroes III entrypoint owning an on-screen
`500x500` untitled window, alongside a `212x152` `Assert Failed` dialog. The
corresponding Steam dump identifies the dialog as the Steam
`CWorkThreadPool::~CWorkThreadPool` assertion. This means a game surface may
already exist underneath the client assertion; a visible assertion dialog
cannot be used as evidence that the game failed to create a window. The next
run must dismiss or suppress that client dialog before collecting the game
surface and level-load evidence.

The standard Steam launch arguments now include `-nofriendsui` in addition to
`-nocrashdialog`. A rebuilt run confirmed that the Friends List surface was no
longer created and the assertion was not visible during the initial handoff;
Steam still reached its normal user-confirmation state. This is a startup-noise
reduction only, not proof of successful gameplay or map loading.

## Purchase gap

No purchase is required for the next validation stages. The owned candidates
already cover D3D9 Effects, Source/multi-depot, two Unreal-era games, a
D3D9 custom engine, keyboard/pointer/controller, audio, save/load, and
Steam Cloud. Recommend a purchase only after those candidates establish a
specific missing engine/API category.

## Persistent Steam login verification — 2026-09-04

`steam login-wait` now distinguishes a live-client success from a persisted
login whose client exits after authentication. The persisted path requires all
of: stored login records, Steam userdata, a connected transport with a
non-zero authenticated account context, and a logged-in UI state
(`OnLoginStateChange` state 5). A stored username alone, stale/incomplete
prefix data, disconnected transport, or absent UI-login evidence still fails
closed. This fixes the observed false negative where Wine Steam had retained
`silvery5d` but `login-wait` reported that the client exited before
verification.

## AoE3 visible-window smoke test — 2026-09-04

Using the provenance-matching `compatibility/experiments/105450-cpu4-standard.conf`
profile, Steam tracked `bin/age3.exe` as a live PE32 process (PID 52587) and
WindowServer exposed an owner-matched `Age of Empires 3` window (640x480).
`macos_focus_capture` captured 12/12 focused frames with no ownership changes;
the first frame visibly contained the Age of Empires III title screen rather
than a black or placeholder surface. A single `Return` key event was posted to
the exact PID and a second six-frame capture remained focused, but all hashes
were unchanged, so this is only a startup/presentation pass and not yet proof
of menu input or map loading. The run was stopped through the AppID-scoped
Macroton cleanup path; peak exact-prefix RSS was 1.57 GiB.

A subsequent repeat with the same profile reproduced a different startup
outcome: Steam created and tracked `age3.exe` (PID 936), but removed it about
six seconds later before a WindowServer game surface appeared. Steam's log
still recorded the `CWorkThreadPool` assertion and the game-process removal;
there was no map-load event. The earlier visible title-screen capture therefore
remains valid presentation evidence, while gameplay/input compatibility is
still unresolved and the profile is not promoted as stable.

The new `compatibility/experiments/105450-shadowalias-wined3d-seh.conf`
profile preserves the prefix's bound Shadowalias runtime while enabling
`+seh,+err,+loaddll,+timestamp`. A diagnostic run reproduced Wine faults before
gameplay: repeated division-by-zero reports at `Steam.exe` address
`0x14000D7CD`, plus a Wine page fault at `0x7BDE1239`; Steam again emitted the
`CWorkThreadPool` shutdown assertion. This attributes the repeat failure to
the Steam/Wine startup path for this run, not to an AoE3 map-load fault, and
keeps the profile experimental.

An opaque-client A/B using `105450-shadowalias-legacy-media.conf` changed only
the Steam client mode and legacy media DLL overrides. In this run the PE32
process remained live for at least 18 seconds, but no owner-matched WindowServer
game window appeared before cleanup; exact-prefix RSS reached about 2.39 GiB
and the run was stopped by the AppID-scoped guard path. This is a longer-lived
startup observation, not a gameplay pass, and the media overrides are not yet
promoted.

The formal Steam-integrated `105450-shadowalias-vulkan.conf` run then reached
the tracked PE32 game process (PID 964) and exposed an owner-matched
`Age of Empires 3` WindowServer surface (640x480). The focused capture passed
10/10 frames with a stable title-screen hash while the Steam client and game
were both live. This upgrades the Steam launch plus Vulkan presentation gate;
menu input, map loading, audio, save/Cloud, and longer stability remain open.

The next attempted input run was correctly stopped before sending any key:
the isolated 105450 prefix presented a real `Sign in to Steam` window instead
of launching the game, so the input gate was not attempted. This is an
interactive-authentication prerequisite, not evidence of a game failure; the
prefix must be reauthenticated in its visible Wine Steam UI before repeating
the input/map-load test.

After the user-facing reauthentication attempt, `login-wait` accepted 105450's
persisted signals (stored login, userdata, connected account context, and UI
state 5), but the subsequent launch still exposed a `Sign in to Steam` window
and did not create `age3.exe`. This is now tracked as a login-session
consistency defect: state-5 log evidence alone is insufficient to prove that
the next Steam launch is authorized. No gameplay claim is made from this run.

The current 105450 Cloud probe reports `state=syncing`, no pending remote
operations, and `launch_ready=false`. No `cloud_files` declaration exists for
the AoE3 profiles, so Macroton cannot yet perform a content-level Cloud
roundtrip for this title; this is intentionally left unverified rather than
inferring Cloud support from Steam's generic sync state.

The full inventory eligibility scan also confirms the target exclusion rule in
the live library: Baba Is You is reported as
`exclude-working-native-mac` despite having Windows/macOS/Linux metadata, so it
cannot enter the legacy fallback candidate set.

A fresh Braid launch attempt (AppID 26800, low-memory Steam profile) did not
reach the game: Steam stopped at `ShowInterstitials` and waited for a user
response before creating the process. No Wine game PID or WindowServer game
surface existed, so no new rendering/input claim is made. The prefix was
cleaned after the bounded attempt.

`steam launch-status` now turns this `waiting-user` state into an actionable
diagnostic: when the exact prefix's Steam client is live it instructs the user
to complete the visible confirmation/interstitial; when no client is live it
labels the event stale and safe to retry. This prevents a generic lifecycle
state from being mistaken for a game launch failure.

Rebuilt binary verification against Braid's current logs produced exactly the
stale-event notice (`Steam is no longer live; this user-response event is
stale and the prefix can be retried`) with no game PID. The new diagnostic is
therefore exercised against a real persisted prefix, not only a unit fixture.

The local AppID 105450 appinfo parser exposes only Windows launch entries and
depots (105451/105452 plus language depots); it exposes no Cloud root/file
mapping. Macroton therefore keeps AoE3 Cloud support unconfigured until a
real save path and Steam's authoritative mapping are observed.

The direct-executable control profile
`105450-shadowalias-direct-seh.conf` bypasses Steam while retaining the same
PE32 payload and Wine runtime. It reached `ConfigDetect.dll` and builtin
`ddraw.dll`, then produced a deterministic 32-bit SEH access violation (read
`0x2DC1` at `0x7BF21239`, handler in `age3.exe`) and exited with status 5.
This separates the failure from Steam's CEF lifecycle: the game itself is also
not currently stable under this runtime, while the earlier title-screen
capture proves that at least one initialization path can present a frame.

The `105450-shadowalias-direct-wined3d-vulkan.conf` A/B reached the same
visible AoE3 title screen under `wined3d-vulkan` (8/8 focused frames, stable
hash), so Vulkan presentation itself is functional. The process also exposed
an `Age of Empires III Error` surface and exited with status 255 after the
bounded interaction attempt; no menu or map-load evidence was produced. The
renderer can therefore present the intro but does not yet establish a stable
gameplay path.

The DXVK D3D9 control profile
`105450-shadowalias-direct-dxvk.conf` did not reach the title screen. Its
result is explicitly recorded as `renderer=dxvk-d3d9-moltenvk` with process
exit status 1; the diagnostic output ends with a nested exception on the signal
stack and no game WindowServer surface was observed. This makes the current
WineD3D/Vulkan title-screen path the least-bad presentation candidate, while
DXVK is rejected for the next interactive run until its early initialization
failure is understood.

The connection-status parser now uses the latest login-state event rather than
an historical `state 5` match. A later logout or intermediate event supersedes
an earlier success, with a dedicated regression test; this narrows the
persisted-login gate after the AoE3 launch inconsistency.

The owner subsequently reported a successful AoE3 launch and normal menu/title
presentation, followed by an immediate exit when a level finished loading.
This is a user-observed gameplay failure, not yet an independently captured
Macroton result: the current result records contain startup/title and direct
SEH failures, but no matching level-load exception from that session. The next
run must therefore use `compatibility/experiments/105450-mapload-seh.conf`,
reproduce the same level-load action, and retain the first `Unhandled`/access
violation line plus the exact exit code before changing renderer or DLL
overrides. Until that evidence exists, AoE3 remains “title/menu proven; 3D
level loading unverified” and no compatibility fix is promoted.

The fresh 2026-09-06 `105450-mapload-seh.conf` run narrowed the failure further:
`age3.exe` stayed resident for about 37 minutes at roughly 49% CPU while the
user-visible state remained on the Logo. The log repeatedly emitted
`Unhandled division by zero at 0x000000014000D7CD` and the Steam work-thread
shutdown assertion. The exact-prefix guard never approached its 3.2 GiB limit;
Macroton stopped the session cleanly after capture. This is now classified as
an exception-loop/logo-stage hang, not slow startup or a resource kill.

A later stored-login AoE3 launch kept the Wine Steam client alive (13 exact
prefix processes, about 1.6 GiB RSS) but produced no new game PID; the latest
console event remained the older title-screen `Game process added` record.
Macroton classified this as a stale running event and stopped the prefix to
release memory. This is session/game-process evidence, not a new 3D gameplay
pass.

The current inventory scan ranks Age of Empires II (2013), AppID `221380`, as
the next Windows-only candidate. Its isolated Macroton prefix is bootstrapped
but currently `sign-in-required` (zero stored login records and zero account
userdata), so it is not safe to launch or use as a renderer comparison yet.
After the owner completes one visible Steam login in that exact prefix, it will
be the next cross-engine/API validation target.

The first 221380 login A/B also exposed a client-bootstrap boundary. The
rendered-login profile exited during CEF/Vulkan initialization before creating
a Wine WindowServer surface; the standard profile kept `Steam.exe` alive but
reported no visible Wine surface and remained disconnected. The corresponding
log shows a 212,314 KiB Win64 Steam client update completing and shutting down
normally before relaunch. The update is now current, but authentication and
gameplay remain unverified; the next attempt must begin with a fresh visible
client launch and must not be inferred from the updater's successful exit.

A subsequent Heroes III Steam launch using the provenance-matching
`297000-bypass-low-memory.conf` submitted successfully, but Steam's latest
event was `ShowInterstitials`; after the client exited, `launch-status` and
`lifecycle-status` correctly reported a stale `waiting-user` state, no live
game PID, and `safe_to_start_interactive=false`. The older `running-observed`
event was not reused as gameplay evidence.

After rebuilding `target/debug/macroton`, the same Braid prefix now reports
`phase=stale-waiting-user` and `safe_to_start_interactive=true` in the real CLI
JSON. This confirms the new state-machine branch is present in the executable,
not only in unit-test code.

Macroton's login gate now requires an explicit updater marker (`Downloading
update`, `Download complete`, package extraction/install, or `Update complete`)
in the bytes appended during the current launch. Generic bootstrap-log growth
from verification or startup is no longer accepted as update progress. This
prevents a live-but-invisible client from being reported as a usable login UI;
the behavior is covered by a CLI regression test.

A fresh real 221380 standard-mode probe after this change returned the bounded
“did not produce a usable login UI or a progressing client update” error and
stopped the exact prefix; it no longer reported the stale
`client-update-progress-observed` state. This confirms the gate change against
the affected Wine Steam prefix, while the underlying disconnected/no-surface
condition remains an external login prerequisite.

The rebuilt binary also emits `launch-status` schema 2 with
`game_process_live=false` for AoE3's historical PID 964 event, and reports
`phase=stale-waiting-user` for Braid's exited interstitial. Clippy and the full
test suite pass after these interface changes.

On 2026-09-04, `cargo fmt --check`, `cargo test --quiet` (196 library tests
and 59 CLI tests), `cargo clippy --all-targets --all-features -- -D warnings`,
and `cargo build --quiet` all passed after adding the per-launch console-log
boundary. The boundary is recorded immediately before Macroton's managed
Steam launch in `.macroton-launch-console-offset`; the next `launch-status`
therefore cannot reuse a `Game process added` line from an earlier run. A
read-only status check on AppID 400 currently finds a live `hl2.exe` host PID
(`game_process_live=true`), but that process predates the new boundary file and
is not counted as a fresh Macroton acceptance run. A new acceptance run must
start through `macroton steam launch 400 ...` and then verify the visible game,
input, rendering, audio, save/reload, and clean exit gates.

The current read-only Portal Cloud baseline is `synced`, `launch_ready=true`,
with all 13 declared files reported `match` and no pending remote operations.
This is a content-comparison baseline only; it does not prove that a newly
created save has uploaded or that a second prefix can restore it.

On 2026-09-05, the AoE3 Cloud probe initially showed `state=syncing` with
`launch_ready=false`, while `steam client-status` reported no fresh client
progress and no AoE3 client was resident. After the parser recognized Steam's
authoritative `Not running AutoCloud. No config data found` event, the rebuilt
CLI now reports `state=no-evidence` (still not launch-ready). This is treated as
an unsupported/unconfigured Cloud feature, not as a renderer or gameplay
failure. Portal was left running in its own exact prefix (14 processes, about
610 MiB RSS), so no second large Wine session was started concurrently.

The follow-up human-readable Cloud inspection identified the exact last event
as `[2026-09-04 21:54:53] [AppID 105450] Starting sync (init,)`, with no
per-account `remotecache.vdf` and no pending client-side operation. This is a
more specific acquisition blocker than a generic `syncing` label; the next
recovery attempt must start the authenticated AoE3 Steam client and allow that
initial sync to complete before map-load diagnostics.

The same log also records `[AppID 105450] Not running AutoCloud. No config data
found` and deletes the per-user `remotecache.vdf`. This is evidence that the
AoE3 Windows build does not advertise Steam AutoCloud data in this client
session, rather than an indefinitely running transfer. Macroton must therefore
classify AoE3 Cloud as unsupported/unconfigured and proceed with local
save/reload validation; it must not block gameplay testing waiting for a cache
that Steam never creates.

Static PE inspection on 2026-09-05 confirmed all three AoE3 entrypoints
(`age3.exe`, `age3x.exe`, and `age3y.exe`) are `i386` and declare the same
legacy surface: Direct3D 9, DirectInput, and WinMM, with legacy DirectX and
Visual C++ prerequisites. This gives Macroton a concrete cross-API test target
once the Cloud/session prerequisite is cleared; it is static compatibility
evidence, not a substitute for running a 3D map.

The next inventory candidate, Mass Effect (AppID `17460`), was checked with
`steam target-status` on 2026-09-05. It returned
`blocked-no-path-bound-win32-entrypoint`: the account has an eligibility review,
but no authenticated Windows payload is installed in the Macroton prefix. It is
therefore not selected for runtime testing yet; installation of its Windows
depot is required before it can join the validation matrix.

`steam cloud-wait` was intentionally not used for this profile: it refuses a
profile without declared `cloud_files`, and AoE3's authoritative Steam appinfo
does not expose a Cloud root/file mapping. This prevents Macroton from waiting
on or comparing invented save paths; AoE3 must first obtain a real mapping from
the authenticated client/session before Cloud roundtrip automation can be
enabled.

On 2026-09-05, the authoritative `steam target-status` check for AoE3
(`105450`) compared the native Steam library with the exact Macroton Windows
prefix and returned `decision=eligible-verified-win32-windows-only` and
`next_action=begin-runtime-validation`. The Windows side is backed by an
installed PE32 entrypoint; the result deliberately remains an eligibility gate,
not proof that AoE3's 3D level loading is stable.

## AoE3 Logo-stage A/B — 2026-09-06

The follow-up `105450-shadowalias-wined3d-seh.conf` run crossed the Steam
handoff gate and created the exact PE32 `bin\\age3.exe` process. It remained
live for about two minutes at roughly 37% CPU and 104 MiB RSS while the visible
state stayed on the AoE3 Logo; no title/menu or 3D map was observed. The fresh
bounded log contained a Wine page fault (`0x7BDE1239`), repeated division by
zero at `0x000000014000D7CD`, and the Steam work-thread shutdown assertion.
Macroton then stopped the exact prefix and verified zero remaining processes.

This fails the map-load/gameplay gate but does not yet identify whether the
primary cause is Steam's handoff or AoE3's graphics/runtime initialization.
The next bounded A/B is a direct `age3.exe` launch with the same prefix/runtime
and no Steam `-applaunch`; only a divergence between those runs can separate
the two causes. Full evidence is recorded in
`docs/evidence/105450-logo-hang-20260906-shadowalias-wined3d.md`.

A same-prefix direct-executable A/B then loaded AoE3's native D3D9 and game
DLLs but exited within the short observation window without a visible surface
or reliable exit code. It is inconclusive for rendering and is not a supported
launch mode for this Steam-protected title; the supported Steam path remains
the only path that reaches a live Logo-stage process. Further work should use
a bounded Steam graphics/compatibility override and require a visible
title/menu capture.

The subsequent bounded `105450-cpu4-standard.conf` run created `age3.exe` but
Steam removed it about seven seconds later with exit code `-1073741819`
(`0xC0000005`, access violation). The diagnostic stream again showed repeated
division-by-zero reports at `0x000000014000D7CD` and work-thread shutdown
assertions. The exact prefix was cleaned and verified at zero processes. This
is now tracked as a reproducible early crash after process creation; it is not
yet a renderer or gameplay pass.

The `105450-builtin-pidgen.conf` dependency A/B did not create a game process:
the isolated Steam client reported `LogonFailure No Connection`. It is
therefore inconclusive for PIDGen and graphics compatibility and remains
experimental; the prefix was cleaned after the bounded run.

## Heroes III HD Edition inventory target — 2026-09-06

Inventory and `steam target-status` identify Heroes of Might & Magic III - HD
Edition (AppID `297000`) as a Windows-only candidate with an exact installed
PE32 entrypoint. A bounded run using
`297000-bypass-wined3d-serialized-standard.conf` started the isolated Steam
client but produced no new Launcher/game process event. This is a Steam
handoff/session observation, not a compatibility failure; the prefix was
cleaned and verified at zero processes. Full evidence is in
`docs/evidence/297000-steam-handoff-20260906.md`.

The game binary also exposes a `noIntroCinematics` switch. Macroton added an
experimental profile passing `-noIntroCinematics`, but the bounded Steam run
did not create a new game process; Steam/CEF remained in client startup. The
switch is not promoted until a visible title/menu capture proves that it
actually bypasses the Logo stage.

A low-memory CEF / `wined3d-opengl` A/B for Heroes III likewise produced no new
Launcher/game event. The client-mode change did not bypass the handoff blocker,
so no Heroes III compatibility claim is made.
### 2026-09-06 AoE3 Steam cold-start warmup

- The default Steam client warmup was raised from 45 to 90 seconds after a
  measured 82.23-second Wine CEF cold start. The change passes 198 library
  tests, 59 CLI tests, clippy, and build.
- A fresh live run with the previous 45-second behavior produced only a new
  Steam `Console Log Start` and no AppID 105450 launch event before Macroton
  submitted/finished; therefore that run is `no-evidence`, not a game crash.
- The existing reproducible AoE3 Logo-stage failure remains separate: the
  game process was created and removed after about seven seconds with
  `0xC0000005`. The 90-second default is intended to remove the Steam-client
  handoff race before re-testing that game boundary.
- The first 90-second live retry had no new `-applaunch` event because the
  isolated Steam session was offline (`LogonFailure No Connection`); this is a
  login/session blocker, not evidence that the Logo-stage crash was fixed.
- A direct diagnostic with `-noIntroCinematics` exited with result code `1`
  and retained the same page-fault/division-by-zero signatures. It produced
  no verified frame and remains diagnostic-only, not a promoted compatibility
  fix.
- A fresh online Steam retry under a 2.4 GiB AppID guard created `age3.exe`
  and Steam removed it after about seven seconds with result code `1`; the
  guard did not fire (peak about 2.30 GiB). No accessible AoE3 window was
  observed, and the same Wine fault signatures remain. This separates the
  formal early exit from the prior 1.0 GiB guard kill.
- The `shadowalias-vulkan` (`wined3d-vulkan`) A/B kept `age3.exe` live for at
  least 108 seconds with no newly observed Wine exception, unlike the
  `cpu4-standard` WineD3D run. However, no accessible or visible game window
  was found, so it is only a stability/backend lead and not a playable pass.
- A follow-up `vulkan-window-debug` profile added Wine window tracing but still
  produced no usable Create/Show window evidence or Accessibility window. The
  process-stability lead therefore remains unpromoted until a visible frame is
  captured.
- A forced-windowed Vulkan retry (`-windowed -width 1280 -height 720`) still
  exited with result code `1` before a usable AoE3 window appeared. Quartz
  enumeration found only Steam/Friends windows and untitled blank Wine
  surfaces; no Age of Empires title/menu frame was observed. The profile is
  therefore not a rendering or gameplay pass, and the exact prefix was cleaned
  back to zero processes.
- A fresh rerun of the known-best `shadowalias-vulkan` profile did expose a
  real owner-matched 640x480 AoE3 window, but direct capture showed the
  authentic Logo unchanged for 30 seconds while `age3.exe` stayed CPU-active
  for about five minutes. This confirms a reproducible Logo-stage hang rather
  than a hidden/black WindowServer surface.
- The Vulkan Steam `noIntroCinematics` A/B passed the argument to the observed
  `age3.exe` command line, yet the captured window remained on the identical
  Logo. The switch is not a fix; the run was cleaned to zero exact-prefix
  processes.
- Macroton's lifecycle inspection now lets a unique fresh exact-prefix game
  process override a stale Steam `waiting-user`/interstitial event and report
  `running-observed`. This corrects observability only; visible rendering and
  gameplay gates remain independent.
- The original-game `NOSTARTUP` A/B was added as an experimental profile, but
  its first bounded Steam attempt produced no fresh launch evidence after the
  90-second warmup. It remains inconclusive until the authenticated Steam
  session submits a new AppID launch.
- The documented Startup/user.cfg route was then tested in the exact 105450
  prefix using `NoStartUp`. The game process and real AoE3 window appeared, but
  the captured frame remained the unchanged Logo. This rules out Steam argument
  forwarding as the primary cause; media/configuration or renderer
  initialization remains the next diagnostic boundary.
- The legacy-media A/B enabled native `dsound`, `msxml4`, and `l3codecx` on the
  same Vulkan/WineD3D Steam path. It still produced only the fixed Logo frame
  in a real AoE3 window, so media DLL overrides alone are not a fix; the exact
  prefix was cleaned.
- The install-tree `bin/Startup/user.cfg` A/B then used the exact
  `noIntroCinematics` spelling exposed by AoE3's own `game.cfg`. It still
  produced the unchanged Logo in a real window, ruling out Startup path/case
  as the primary cause; the exact prefix was cleaned.
- A standard-client SEH/DLL trace attributed the repeated
  `0x14000D7CD` division-by-zero reports to the Windows `Steam.exe` image
  (`0x140000000` base), not to `age3.exe`. This is a Steam/CEF lifecycle fault
  boundary and reinforces keeping the opaque-software client path separate
  from AoE3 graphics diagnosis.
- The follow-up direct D3D diagnostic profile
  `105450-shadowalias-direct-d3d-debug.conf` loaded the game's native
  `bin\\d3d9.dll`, `granny2.dll`, and `binkw32.dll`; Wine logged only D3D DLL
  initialization and no device creation/present calls during the bounded run.
  This is consistent with an AoE3 startup/renderer initialization hang before
  the playable scene, not a missing dependency. The direct process was stopped
  and the exact-prefix resource status returned zero RSS/processes.
- A true WineD3D comparison then used the managed `macroton-winehq-macos`
  runtime rather than the shadowalias runtime (whose `d3d9.dll` is DXVK).
  WineD3D/OpenGL reached the Apple M4 GL adapter and exited on an access
  violation; WineD3D/Vulkan reached MoltenVK but failed on unsupported
  `VK_EXTERNAL_MEMORY_HANDLE_TYPE_MTLBUFFER_BIT_EXT` buffer creation. A new
  Macroton runtime variant combining the existing Wine 11.14 tree with the
  checked-in `0002`/`0003`-built `win32u.so` removed that Vulkan error, but the
  direct AoE3 process then remained CPU-idle with no window for about a minute.
  None of these experiments reached a menu; all were stopped and resource
  status returned zero processes.
- The patched WineD3D-Vulkan runtime was then tested through the real Steam
  launch path. Windows Steam itself stayed live, but its CEF helper repeated
  `Steam.exe` division-by-zero reports and no `age3.exe` launch event appeared
  during the bounded warmup. This is a separate Steam-client blocker; the
  prefix was restored to the known shadowalias runtime after the test.

- Heroes III (297000) SDL2 2.30.9 plus i386 ANGLE replacement in a temporary
  payload reached the real game process but not a playable window; Steam CEF
  remained the dominant memory consumer at about 3.9 GiB. A follow-up
  pre-warmed-IPC direct launch with explicit `SteamAppId`/`SteamGameId` kept
  `HOMM3 2.0.exe` alive for more than 90 seconds and logged `Steam: appID:
  297000`, proving a viable independent-process boundary. This is still not
  gameplay evidence until a visible 2D menu and input are reviewed.
- Braid (26800) was used as a cross-engine 2D control. The same pre-warmed-IPC
  direct path exited after about two seconds with code 5 and a Steam
  `CWorkThreadPool` assertion, with no sustained game process. Therefore the
  IPC-direct path is game-specific and must remain diagnostic; Braid's
  supported profile remains the normal `launch=steam` path.
