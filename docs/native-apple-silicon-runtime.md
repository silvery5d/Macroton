# Native Apple Silicon runtime investigation

## Why this became a product gate

Alan Wake reaches a real 3D scene through the public WineHQ 11.14 runtime, but
the Wine host executable and its Unix-side libraries are x86_64-only Mach-O.
On Apple Silicon, Wine's windowing and WineD3D/OpenGL work therefore execute
inside Apple Rosetta as well as the Win32 game code. A ten-second 720p test
saturated roughly one host CPU core and changing WineD3D command serialization
did not materially change that result.

`macroton runtime probe-win32` now reports the host-execution class. The current
runtime passes functional PE32 execution while reporting:

```text
Host:    host translation required
```

That distinction is mandatory in future runtime acceptance: command-line
compatibility is not a 3D performance pass.

## Upstream boundary

Wine 11's new WoW64 design can run a 32-bit Windows program without a 32-bit
Unix process. Wine's ARM64/ARM64EC work also defines interfaces through which
an external x86 emulator can execute guest code while Wine code remains native.
Wine itself does not ship that emulator.

FEX currently implements Wine's emulation interfaces and supports 32-bit and
64-bit Windows game use cases, but its documented host is ARM64 Linux. It is
not a drop-in macOS runtime. Consequently, simply rebuilding Wine's Unix host
as arm64 would run ARM Windows binaries but would not make Alan Wake's PE32
i386 code executable.

## Macroton decision

Keep the public x86_64 WineHQ package as the functional bootstrap and as a
fallback for light Win32 titles. Do not market it as the final high-performance
Apple Silicon runtime.

The independent long-term runtime track must prove all of these together:

1. native arm64 macOS Wine Unix/windowing/graphics modules;
2. a redistributable x86/i386 emulation component implementing Wine's interface;
3. 32-bit x87/SSE correctness for legacy games;
4. a Metal-capable D3D9 path that does not rely on CrossOver or D3DMetal;
5. Steam, input, audio and Cloud parity in an isolated AppID prefix;
6. scene-level performance evidence against the current x86_64/OpenGL baseline.

Until those gates pass, Alan Wake is classified as `runs, not playable`, and
its 720p/serialization profiles stay experimental.

## Wine 11.14 arm64 architecture proof (2026-08-12)

Macroton can build and install an arm64 macOS Wine host from unmodified Wine
11.14. The resulting architecture split is correct:

```text
bin/wine                         Mach-O arm64
lib/wine/aarch64-unix/ntdll.so   Mach-O arm64
lib/wine/aarch64-windows/cmd.exe PE32+ Aarch64
lib/wine/i386-windows/cmd.exe    PE32 i386
```

Execution is not yet viable: even the native AArch64 `cmd.exe` probe is killed
immediately with exit status 137. This is consistent with upstream Wine bug
58567, where the arm64 loader's 4 KiB segment alignment and page-zero layout
produce a malformed Mach-O on macOS. Upstream additionally identifies the
fixed `USER_SHARED_DATA` mapping at `0x7ffe0000` and preservation of register
`x18` as platform requirements. Removing the linker options merely exposes the
fixed-address mapping failure, so it is not a usable workaround.

The upstream issue remained open through July 26, 2026, when another report
was marked as its duplicate. Native arm64 Wine plus FEX/Box64 is therefore not
currently a drop-in macOS escape hatch: FEX, Box64, and Hangover document their
production hosts as ARM64 Linux, while Wine's native macOS host remains blocked
before an x86 emulator DLL can solve the guest side.

The reproducible proof recipe lives in
`scripts/build-runtime-macos-arm64.sh`. It establishes that Macroton does not
need CrossOver to produce the architecture, while keeping the upstream macOS
loader defect and the still-missing i386 emulator as explicit product gates.

Use `macroton runtime audit ENTRYPOINT` to test these gates independently. Its
readiness result is one of:

- `translated-bootstrap`: Win32 works, but the host itself needs Rosetta;
- `native-host-blocked`: even the matching ARM64 Windows payload cannot run;
- `native-host-needs-x86-emulator`: the native payload runs but PE32 does not;
- `native-win32-ready`: both the native host and PE32 execution pass.

Only `native-win32-ready` is eligible for the final Apple Silicon game runtime.
