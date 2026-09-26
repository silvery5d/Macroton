# Macroton Runtime build

Macroton's release runtime must not contain or load CrossOver binaries. The
baseline is built from the public WineHQ `wine-11.14` tag mirrored at
`wine-mirror/wine`. The source archive is pinned by SHA-256 before extraction.
Run `scripts/test-no-crossover-runtime.sh` alongside the release audit; it
checks effective runtime/profile assignments and ignores explanatory comments.

## Host inputs

- Apple Silicon macOS with Rosetta 2
- Xcode command line tools
- Homebrew `bison` and `mingw-w64`
- Macroton's checked-in `scripts/build-runtime-macos.sh`

The current recipe produces an x86_64 macOS Wine host and both i386 and x86_64
Windows PE modules using Wine's new WoW64 layout. This is a functional
bootstrap, not the final Apple Silicon architecture: the Wine Unix and graphics
layers themselves run through Apple Rosetta. Runtime probes report this as
`host translation required` so a passing Win32 command-line probe cannot be
mistaken for a native or game-performance pass.

The release target is a native Apple Silicon Wine host with an explicit x86
emulation interface for the Windows payload. Wine 11 supports the architecture
separation required for that design, but Wine does not bundle the x86 emulator;
the emulator and graphics path must therefore be pinned, licensed, packaged,
and benchmarked as separate Macroton runtime components.

Run:

```sh
brew install bison mingw-w64
./scripts/build-runtime-macos.sh
```

The first mandatory gate is:

```sh
cargo run -- runtime probe-win32 \
  /private/tmp/macroton-wine-11.14/install/bin/wine
```

A release candidate must additionally record compiler versions, archive the
install tree, create a runtime manifest with that archive's SHA-256, pass the
Steam connectivity/login flow, and launch the Silver main menu on a machine
where CrossOver is absent.

For each real-game promotion, `macroton evidence verify-independence` adds a
live-process check to the build/runtime provenance. It requires mappings from
the selected managed runtime and AppID prefix while rejecting CrossOver and
CodeWeavers paths. This complements archive/source provenance; neither check
alone proves the other.

Before packaging a managed runtime, run the static release-tree gate:

```sh
macroton runtime release-audit runtimes/macroton-winehq-macos-11.14.conf \
  --runtime-root /path/to/installed-runtimes --json
```

It requires a checksum-matching managed archive with an HTTPS source and LGPL
SPDX declaration, scans every installed path and symbolic-link target, and
audits the Wine entrypoint's Mach-O load commands. It rejects CrossOver,
CodeWeavers and D3DMetal identifiers, escaping links and unsupported filesystem
entries. This is a structural packaging gate; full source provenance and the
live `verify-independence` mapping remain separate evidence.

The current pinned WineHQ installation produced the redacted record at
`docs/evidence/runtime-release-audit-20260813.json`: 5,398 filesystem entries,
223 internal symbolic links and one Wine entrypoint load dependency were
checked against manifest fingerprint
`63ceb7633c0e477d2bd5b1057a38e539076d14678ded0f648706936492b5a400`.
This is current-tree evidence, not a substitute for the clean-host live game
mapping gate.

Formal compatibility profiles receive a second gate. Files directly under
`compatibility/` (excluding `*.example.conf`) must pass:

```sh
macroton runtime profile-release-audit compatibility/606680.conf \
  runtimes/macroton-winehq-macos-11.14.conf \
  --runtime-root /path/to/installed-runtimes \
  --windows-steam-root /path/to/authenticated/windows/Steam --json
```

This binds the profile to that audited runtime and a path-bound real PE32 game
entrypoint. It accepts either an authenticated Windows install or the profile's
pinned native-Steam depot-content layout; the latter is audited through the
same ready launch plan and is reported as
`candidate-verified-win32-depot-content`. It requires Steam launch, Standard
client mode, explicit renderer/API, BuildID and every depot manifest.
Commercial-runtime identifiers in runtime, renderer, arguments or environment
are rejected. Research profiles remain under `compatibility/experiments/` and
are not release profiles merely because they parse or launch.
Native-Mac viability remains a distinct eligibility gate: PE32 proof cannot
override a working official Mac build such as Baba Is You.

Schema 2 also emits `windows_payload_root`, exact `entrypoint_architecture` and
the entrypoint SHA-256. This makes an audit sensitive to switching between an
installed tree and a no-copy depot, to a 64-bit executable substitution, and to
payload-file drift even when filenames remain unchanged.

The current five formal profiles passed against the pinned WineHQ tree; Silver
uses the native-Steam no-copy depot gate while the other four profiles use the
authenticated Windows-install gate. Their
AppID/profile hashes, renderer/API and depot counts are preserved in
`docs/evidence/release-profile-audit-20260813.json`. The record deliberately
retains all gameplay dimensions in `not_inferred`.

The current local candidate manifest is
`runtimes/macroton-wine-11.14.local.conf`. Its `file://` source and checksum
identify the locally built artifact; a public release replaces that source
with the project's download URL and updates the checksum to the published
artifact.

## Steam-capable public runtime

Macroton also pins a public WineHQ-compatible macOS package in
`runtimes/macroton-winehq-macos-11.14.conf`. It is built from the same Wine
11.14 source release and bundles the open-source x86_64 runtime dependencies
needed by Steam, including FreeType, GnuTLS, SDL2, and MoltenVK. Macroton
downloads it from the public GitHub release, verifies its published SHA-256,
and installs it as a managed archive. It contains no CrossOver binaries or
commercial D3DMetal components.

The source-only recipe above remains the provenance and compiler smoke test;
the pinned public package is the current product runtime while the dependency
bundle is made fully reproducible inside this repository.

The successful test host also required the environment-gated source patch under
`patches/wine-11.14/` to route selected Steam CM DNS names around a local Clash
Fake-IP resolver. The patch is retained for exact experiment reproducibility,
but it is not enabled by the default build and does not belong in a release
profile. A production DNS preflight must replace the host-specific address
override.

## Experimental D3D9-to-Metal runtime

Alan Wake's accepted fallback still uses WineD3D/OpenGL and is not playable in
its real 3D scene. The independent performance path is a source-built Wine
11.14 runtime plus patched DXVK 1.10.3 and MoltenVK. It does not use CrossOver
or D3DMetal.

The runtime applies `0002` and `0003` from `patches/wine-11.14/`, builds DXVK
with `scripts/build-dxvk-d3d9-moltenvk-experiment.sh`, and places
`libMoltenVK.dylib` beside `win32u.so` under
`lib/wine/x86_64-unix/`. Relying on a launcher `DYLD_*` variable is not an
acceptable package design: it did not reliably propagate into Wine's spawned
processes. The patched `win32u.so`, MoltenVK binary and DXVK `d3d9.dll` must be
pinned in a component manifest and reverified whenever the runtime is resolved.
The base can be the complete public WineHQ macOS package so Steam CEF,
authentication and the game use one Wine build while Macroton owns the graphics
substitutions.

Build the matching patched Wine `win32u.so` from the checksum-pinned Wine 11.14
source with:

```sh
./scripts/build-patched-wine-win32u-macos.sh /path/to/libMoltenVK.dylib \
  /path/to/wine-build-cache
```

This recipe deliberately excludes the host-specific DNS override patch.

Compose a non-overwriting runtime tree with:

```sh
./scripts/compose-dxvk-d3d9-moltenvk-runtime.sh \
  /path/to/wine-root /path/to/patched/win32u.so /path/to/d3d9.dll \
  /path/to/libMoltenVK.dylib dxvk-1.10.3-moltenvk-1.4.2 \
  /path/to/new-runtime-root
```

Alan Wake exposed an upstream SPIRV-Cross Metal argument-buffer bug in the
exact revision pinned by MoltenVK 1.4.2. Two sampled-image variables at the
same Vulkan binding correctly generated a texture alias, but the synthetic
comparison sampler alias was omitted. The generated MSL therefore referenced
`s0_2d_shadowSmplr` without declaring it. Macroton retains the narrow source
patch at
`patches/spirv-cross/0001-msl-alias-overlapping-combined-samplers.patch` and
the captured shader fixture at
`tests/fixtures/alan-wake-shadow-sampler.spvasm`.

The reproducible builder pins MoltenVK commit
`db66022459ffb663aa2b50f6b018bc2e124f5edf` and SPIRV-Cross commit
`6c09849fe88c48eaed08413aa022aaa136a3a057`. It fetches dependencies before
applying the patch because MoltenVK's fetcher hard-resets dependency checkouts,
then rebuilds the patched static library before packaging MoltenVK:

```sh
./scripts/build-patched-moltenvk-shadowalias.sh /path/to/MoltenVK-v1.4.2
```

For a fast source-level regression check without rebuilding MoltenVK, run:

```sh
./scripts/verify-spirv-cross-shadow-alias.sh /path/to/patched/SPIRV-Cross
```

The regression gate assembles the captured SPIR-V, compiles SPIRV-Cross and
requires all three missing comparison-sampler aliases in generated MSL. It is
a code-generation gate, not a game-playability gate. Disabling
`MVK_CONFIG_USE_METAL_ARGUMENT_BUFFERS` is not a workaround: the same shader
then declares ordinary and comparison textures/samplers at duplicate discrete
Metal resource indexes, which Metal rejects.

Portal reproduced the same missing comparison-sampler alias. With the narrow
patch applied, an authenticated AppID 400 Steam launch reached the first 3D
chamber without the prior pipeline failures. This is experimental live-game
evidence, not release provenance: the tested composed runtime lived outside the
managed release catalog and its payload view came from an isolated clone of
historical authenticated content rather than a fresh download.

An authenticated per-AppID prefix must never silently change runtimes. After
registering the patched runtime, the explicit migration gate is:

```sh
macroton steam rebind-runtime APPID --profile PATH \
  --from-runtime-fingerprint OLD_SHA256 \
  --runtime-root /path/to/runtime-registry \
  --data-root /path/to/macroton-data
```

The command requires an idle prefix, the exact old runtime fingerprint and a
verified target runtime. It updates runtime identity while preserving the
prefix's Steam account and userdata; it does not copy login data between
AppIDs. A failed marker write rolls the identity change back.

The standalone gate is:

```sh
MACROTON_PROBE_ATTEMPTS=1 MACROTON_WINEDEBUG=warn+vulkan \
  ./scripts/probe-patched-dxvk-d3d9.sh \
  /path/to/runtime/bin/wine /path/to/d3d9.dll /path/to/isolated-prefix
```

A passing probe means CreateDevice, Clear and Present reached Metal. It does
not establish game entitlement, 3D performance, input, audio, save, Cloud or
stability. The profile name `dxvk-d3d9-moltenvk` therefore fails closed unless
the installed runtime contains a verified component with the matching prefix.

For a real-game black-presentation case, capture MoltenVK's first submitted GPU
frame instead of inferring GPU work from a live process or swapchain log:

```sh
cargo build
scripts/capture-moltenvk-first-frame.sh \
  APPID compatibility/experiments/APPID-dxvk.conf \
  /path/to/windows/Steam /path/to/runtime-registry /path/to/data-root \
  /private/tmp/macroton-APPID-first-frame.gputrace \
  /private/tmp/macroton-APPID-first-frame-evidence.json 60
```

The script follows MoltenVK's documented automatic-capture environment,
requires an idle exact AppID prefix, refuses to overwrite either output, bounds
the wait to 1-300 seconds and stops only the exact AppID prefix on exit by
default—including launch failures after the idle check. Before cleanup it runs
`evidence verify-gpu-capture`, which requires the same live game
PID/profile/runtime/session, rejects a changing capture tree across a stability
interval and records a deterministic tree hash, file count and byte count. Set
`MACROTON_KEEP_RUNNING=1` only when interactive follow-up is intentional. A
verified `.gputrace` proves GPU command recording; it does not by itself prove
visible presentation.
Run `scripts/test-capture-moltenvk-first-frame.sh` to exercise successful
capture/evidence, bounded timeout cleanup, busy-prefix refusal, launch-failure
cleanup and existing-output refusal with a fake Macroton executable; all
successful or post-launch failure paths must record the exact AppID stop.
