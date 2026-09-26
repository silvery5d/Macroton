# Diagnostic evidence tools

These small programs support compatibility investigation. They are not part of
the runtime and do not alter Steam ownership or authentication.

## Windows probes

Build PE64 probes with the public MinGW toolchain:

```sh
x86_64-w64-mingw32-gcc -O2 -Wall -Wextra -Werror \
  tools/windows/window_probe.c -o /private/tmp/macroton-window-probe.exe
```

`window_probe.exe` enumerates top-level Win32 windows and can activate, resize,
send a key, move the pointer, or send a timed left click to a title. Its input
actions are useful only after a visible game window exists; they do not prove
that the target control handled the input. A changed game frame is required as
the response evidence.

`network_probe.c` and `http_probe.c` distinguish raw Winsock and HTTP behavior.
`cdp_probe.c` inspects a Steam CEF target through its already-open local DevTools
socket and can capture that target. CDP evidence proves CEF rendering, not the
game's WindowServer presentation. It may be used to expose a legitimate Steam
QR login page when Wine's WindowServer surface is black. Treat that QR capture
as short-lived authentication material: show it only to the account owner,
never commit it, and do not interpret successful CEF rendering as a completed
login.

Build its PE64 executable reproducibly with:

```sh
scripts/build-cdp-probe.sh /private/tmp/macroton-cdp-probe.exe
scripts/test-build-cdp-probe.sh
scripts/find-steam-login-cdp-target.sh
scripts/test-find-steam-login-cdp-target.sh
scripts/capture-steam-login-qr.sh APPID /absolute/path/to/wine
scripts/test-capture-steam-login-qr.sh
```

For a black Wine WindowServer login surface, the native macOS mirror can keep
retrying until a real QR-sized image arrives and display it in an AppKit window:

```sh
scripts/mirror-steam-login-qr.sh APPID /absolute/path/to/wine
```

The mirror uses only the short-lived QR image in `/private/tmp`, rejects the
login page's loading spinner, and removes its temporary files when stopped.

The script uses the public MinGW toolchain and disables the PE timestamp so two
builds from the same source compare byte-for-byte. The executable remains a
diagnostic outside the managed runtime and contains no credentials or account
state.
The target selector accepts only the fixed localhost port, an exact `page`
target titled `Sign in to Steam`, and exactly one matching WebSocket URL. It
prints only `/devtools/page/...`, never the full target inventory or page URL.
The capture wrapper builds the probe into `/private/tmp`, keeps the CDP response
in memory, writes the PNG to a random `/private/tmp` file with mode `0600`, and
removes partial output on any failure. Delete the successful PNG immediately
after the owner scans it; it is authentication material, not compatibility
evidence.

## macOS probes

Build the WindowServer capture helper with:

```sh
clang -O2 -Wall -Wextra -Werror \
  -framework ApplicationServices -framework CoreGraphics \
  tools/macos/window_capture.c -o /private/tmp/macroton-window-capture
```

Use `--list` to obtain current window IDs, then pass a window ID and output PNG
path. This captures the actual macOS-composited game window. `image_mirror.swift`
was used only to make an otherwise black CEF login surface visible during
legitimate interactive authentication.

For an input-response check, build the bounded native mouse probe with:

```sh
clang -O2 -Wall -Wextra -Werror -framework ApplicationServices \
  tools/macos/input_probe.c -o /private/tmp/macroton-input-probe
```

`input_probe --click X Y` emits one timed left click at an explicit macOS screen
coordinate. Pair it with before/after WindowServer captures; the command alone
does not prove the game handled the event.

`macos_interaction_probe.swift` is the stronger integrated path. It resolves and
focuses one exact window/PID, verifies that the target is the topmost visible
WindowServer surface at its center (Wine may report a different host PID as the
macOS frontmost application), captures `before.png`, posts one allowlisted key or
one window-relative click, waits a bounded interval, captures `after.png`, and
emits a flat JSON record containing PNG/sample hashes, luma and changed-pixel
ratio. It refuses locked sessions and existing output directories. Pair the
result with `macroton evidence verify-interaction`; review the two images on a
known static menu before treating the associated change as semantic input.
Pass `--write-json` before the action to atomically create `interaction.json`
inside the capture directory after both PNGs exist. The one-session capture
script uses this mode so a missing shell redirect cannot discard an otherwise
valid record; absence of the JSON remains an unambiguous incomplete probe.

For a single targeted keyboard event, build `macos_key_probe.swift` with AppKit
and pass an exact game PID plus one supported key name. The probe activates that
application and posts only one key-down/key-up pair to the PID. As with mouse
input, a changed game frame is the response evidence; successful posting alone
is not.

When Wine Steam is alive but its native surface was restored off-screen, use the
window locator's recovery mode with the exact Steam PID:

```bash
macos_window_locator PID --recover
```

This only raises accessible windows owned by that PID and moves them to a
centered rectangle on the main display. It does not terminate processes, alter
the Wine prefix, or touch Steam login data. If Accessibility is unavailable,
the command reports that limitation and no recovery is claimed.

For a read-only CEF inspection that does not require a visible Steam window,
run `scripts/steam-cdp-inspect.js`. It only connects to the local
`127.0.0.1:8080` endpoint and prints one JSON record per page target, including
the page title, URL, visible text, and button labels:

```bash
scripts/steam-cdp-inspect.js
```

For focus-sensitive temporal defects such as flicker, build
`macos_focus_capture.swift` with AppKit, CoreGraphics, CryptoKit, ImageIO, and
UniformTypeIdentifiers. It captures one exact WindowServer window at a bounded
interval. Before sampling it activates the exact expected PID, then writes a
CSV row and SHA-256 for every PNG and fails unless that PID owns both the window
and the foreground application for every sample. This prevents an unfocused
capture from being promoted as focused-game evidence even when invoking the
diagnostic command initially changes focus. If AppKit activation is insufficient
for a Wine window, the tool asks System Events to foreground the exact PID. Its
last fallback verifies a visible layer-zero window and emits one bounded click
in its title-bar area before sampling; it never clicks the game content area.
The emitted CSV uses the verifier's canonical
`frame,elapsed_ms,frontmost,average_luma,sampled_sha256,window_id,pid` schema;
the PNG path is deterministic as `frame-NNNN.png` in the supplied output
directory.

`macos_focus_stream.swift` is the higher-rate companion for defects that a PNG
loop can miss. It binds a ScreenCaptureKit stream to one exact window and PID,
requests 1--120 fps, and emits per-frame elapsed time, average luma, and a hash
of spatially sampled BGRA pixels. It fails closed if any delivered frame was
captured while another application was frontmost, if a frame is invalid, or if
the window owner changes before the stream ends. Each CSV row includes the
exact window ID and PID so `macroton evidence verify-focus-stream` can bind the
capture to the live Steam/profile/runtime session instead of trusting the probe
invocation. It also refuses to begin when
the interactive macOS session is at `loginwindow`, because a game cannot become
the true foreground application while the screen is locked. It writes no image files and
is intended for whole-frame flash and cadence diagnosis; localized artifact
review still uses `macos_focus_capture.swift` PNGs.

For a human-reviewed continuously moving scene, pass the preserved CSV to
`macroton evidence verify-motion-rate`. That verifier derives only a sampled
motion-presentation proxy (change rate and freeze intervals) and binds it to the
live game session. It intentionally does not rename ScreenCaptureKit delivery
or sampled hash changes as engine FPS.

For a bounded audio-presence check, build the ScreenCaptureKit probe with:

```sh
swiftc tools/macos_audio_probe.swift -o /private/tmp/macroton-audio-probe
/private/tmp/macroton-audio-probe WINDOW_ID PID 10 -50
```

`macos_audio_probe.swift` resolves one exact WindowServer window and expected
owning PID before capture, binds a ScreenCaptureKit filter to that window, and
reports aggregate PCM buffer/frame counts, RMS and peak dBFS, and the proportion
of buffers above the supplied silence threshold. It fails when no buffer rises
above that threshold, no audio buffer arrives, a PCM buffer is invalid, the
format changes, or window ownership changes.
Screen Recording permission is required.

A successful probe is deliberately only **audio-presence evidence** for the
filtered application/window stream delivered by ScreenCaptureKit. It does not
prove that the music/effects are the right content, that their mix or spatial
position is correct, that there are no clicks/dropouts, or that a human can hear
the result at the current system volume. The dBFS threshold is an explicit test
parameter, not a universal compatibility threshold; pair the JSON with a known
in-game action and a human listening pass before promoting a game's audio gate.

`macos_audio_fixture.swift` is a short-lived, low-volume 440 Hz macOS fixture for
testing the probe itself. It prints its PID and window ID, then exits after the
requested 2--60 seconds. A successful fixture capture validates the local
ScreenCaptureKit/PCM measurement path only and must never be recorded as evidence
that a Windows game, Wine audio, or a Macroton profile works.

All captures should be paired with SHA-256 hashes, process/module evidence, and
the exact runtime/profile fingerprints. A process, a launch request, or a CEF
frame alone is not a playable-game result.

## Short-lived Wine PE address probe

`wine_pe_module_probe.c` is a read-only Win32 diagnostic for a short-lived
managed game child. Build it as i386 and run it inside the exact AppID prefix
before the Steam launch:

```sh
i686-w64-mingw32-gcc -O2 -Wall -Wextra -Werror \
  -o /private/tmp/macroton-wine-pe-module-probe.exe tools/wine_pe_module_probe.c
WINEPREFIX="/absolute/path/to/exact/appid/prefix" \
  /absolute/path/to/registered/wine \
  /private/tmp/macroton-wine-pe-module-probe.exe \
  'Game.exe' 0x78800398 180
```

The optional wait is 1--300 seconds (default 60). The tool reports the
target process's Toolhelp module snapshots plus `VirtualQueryEx` region and
changed `ReadProcessMemory` bytes at the requested 32-bit address; it exits
after the observed child terminates. It does not attach a debugger, write
process memory, or prove module ownership when Toolhelp does not cover the
address. Keep the normal Macroton exact-prefix RSS guard active and stop the
same prefix with `--preserve-session` after collecting the result. A live
address snapshot is not a gameplay or normal-exit pass.

## Visible SteamCMD login

When an interactive SteamCMD session would otherwise be hidden inside an agent
terminal, open a user-visible Terminal window with:

```sh
scripts/open-steamcmd-login-terminal.sh 205100
```

If Apple Events cannot open Terminal automatically, print the same safe
command for manual execution:

```sh
scripts/open-steamcmd-login-terminal.sh --print-command 205100
```

Type `login ACCOUNT_NAME` at the Steam prompt, enter the password only at
SteamCMD's hidden prompt, complete Steam Guard, then type `quit`. The launcher
never receives or stores the password.
The Rust CLI also refuses to start `login` or `acquire` when stdin is not a
real TTY, preventing hidden agent sessions from waiting indefinitely for
credentials.

After authentication, use a clean visible acquisition window for a profile-pinned
Windows depot:

```sh
scripts/open-steamcmd-acquire-terminal.sh 205100
```

That helper runs `steamcmd acquire` for the matching preflight profile and does
not put credentials on the command line.
