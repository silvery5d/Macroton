# Macroton 2D target matrix

The current product milestone is limited to legacy 2D Win32 Steam games. A
candidate is promoted only after visible 2D rendering, input, audio, save/load,
and sustained-playability evidence are collected. Steam Cloud roundtrip is
required only when the game supports Steam Cloud; otherwise its absence must
be recorded explicitly.

## Inventory-first shortlist (2026-09-07)

| AppID | Title | Engine / API class | Local evidence | Decision |
|---:|---|---|---|---|
| 606680 | Silver | SDL + OpenGL; SDL input/audio; WinMM | PE32/i386 `silver.exe`; Windows depot installed; actual interactive 3D/menu scene observed | **3D side lane; exclude from primary 2D coverage** |
| 297000 | Heroes of Might & Magic III - HD Edition | SDL/UI renderer + legacy DirectX prerequisites; mouse/keyboard; OpenAL/WinMM | Windows-only store review; installed depots 297001/297002; exact PE32/i386 launch entrypoint verified in its isolated prefix. Genuine Launcher Play reaches the game within a 2 GiB guard using low-memory Steam mode, but the game exits code 3; `FULLSCREEN=0`, `RESOLUTION=800x600`, and a completed one-variable SDL `opengles2` control each failed to cure it | **Primary launcher/UI 2D target; gameplay unverified** |
| 613830 | CHRONO TRIGGER | PE32/i386 Cocos2d game; GLFW/WGL OpenGL window creation, WinMM and OpenAL audio | Build 11890051 installed; Wine Devel 11.14 exposes zero WGL formats, while independent Wine Staging 11.14 exposes 720 formats with a valid selection. Steam-launched child repeatedly exits `0xC000001D` at `0x78800398`; a read-only live probe saw that address transition into private executable memory outside enumerated PE modules. No playable window observed. | **Eligible Win32 2D target; WGL gate passed with Staging, game startup still blocked; gameplay and Cloud roundtrip unverified** |
| 26800 | Braid | SDL/OpenGL or D3D9 2D platformer | Windows depot is installed; the official Mac build is 32-bit and unusable on current macOS. The 2026-10-21 eligibility review records the owner's correction and Steam's 32-bit support cutoff. | **Eligible legacy-Mac fallback candidate; gameplay still unverified** |
| 221380 | Age of Empires II (2013) / Retired | Isometric 2D RTS; D3D9/D3DX9, DirectSound/WinMM | Local account history includes this Windows-only title, and an earlier exploratory payload had a PE32/i386 `aok hd.exe`; the current independent AppID 221380 prefix has no installed payload or stored login. `eligibility/221380.conf` now records the reviewed Windows-only 2D scope. | **Inventory-first next candidate, not current installation or gameplay evidence; authenticate and reacquire before PE recheck** |
| 1546570 | 仙剑奇侠传 (original) | PAL98 2D game; SDL2 audio/input | Installed Steam entrypoint `PalWorks.exe` and playable `PAL98/PAL.exe` are PE32+ x86-64; bundled `DOSBox.exe` is PE32/i386 | **Experimental Win64 2D lane; exclude from Win32 coverage** |
| 2280 | DOOM + DOOM II | Modern source-port/3D renderer | Windows-only inventory candidate, but unsuitable for the current 2D gate | Defer |
| 955900 | Amazing Cultivation Simulator | Engine and renderer not classified | Windows-only candidate, but engine/rendering classification is not yet bounded | Defer pending classification |

## Inventory gap and low-cost next candidate (2026-09-26)

A fresh `steam inventory --2d-only --data-root ...` scan of the local Silver
account history had returned Silver and Heroes III as installed PE32/i386
targets, CHRONO TRIGGER as owned but uninstalled, and Sword and Fairy as a
Windows-only 2D record whose main playable executable is PE32+ rather than
PE32. The Silver result was a scope misclassification: earlier real-game
evidence explicitly records an interactive 3D/menu scene. Its eligibility
review is now `scope=3d`, so the strict 2D list must exclude it. Account-history
evidence is not a fresh entitlement check.

The first external candidate is **Peggle Extreme (Steam AppID 3483)**, which
the [official Steam page](https://store.steampowered.com/app/3483/Peggle_Extreme/)
lists as free-to-play, Windows-only, 2D puzzle action with a DirectX 7 minimum
requirement and no advertised Steam Cloud feature. This would exercise an
older DirectX/PopCap class distinct from Heroes III's SDL/ANGLE path without
asking the owner to buy a game. It is **not** currently
in the verified local inventory; the page does **not** prove that the actual
Steam executable is PE32. Before admitting it to the Win32 matrix, the owner
must add it to their Steam account, Steam must download its Windows depot, and
Macroton must inspect the executable's PE machine type. If it is not PE32/i386,
exclude it rather than counting it as a Win32 result. No game-compatibility
claim is made for this candidate yet.

The currently verified PE32/i386 installed strict-2D inventory contains
Heroes III HD Edition and CHRONO TRIGGER. CHRONO TRIGGER is installed and
its game entrypoint is PE32/i386. Its first Wine Devel launch failed at WGL
window creation. A separate independent Wine Staging runtime passes the same
WGL gate, but the Steam-launched game then exits with an illegal instruction
before gameplay. The installed original 仙剑奇侠传 is useful for
2D runtime diagnostics, but its 64-bit game executable is not Win32 evidence.
Silver's macOS Steam listing is not treated as a viable native alternative:
the locally installed `Silver.app/Contents/MacOS/Silver` is a thin Mach-O
`i386` executable (SHA-256
`096a9150febe267721dc007148b75de2ddafefefa4138399b175beab50fa0803`),
with no x86_64/arm64 slice. The official store still lists a legacy macOS
minimum of OS X 10.7; the local binary is the decisive modern-macOS gate.
The 2026-09-26 combined `steam inventory --eligibility-dir eligibility
--verified-win32-only --2d-only` scan of the stored Silver account history
returns Heroes III and CHRONO TRIGGER. Silver still has PE32 evidence, but
its observed 3D gameplay excludes it from this strict 2D list. This remains
local history plus installation/review evidence, not a fresh entitlement
check or game-compatibility verdict.
Without `--data-root`, a scan rooted in Silver's prefix can see only Silver's
installed payload. The inventory result remains local history, not proof of
current entitlement or gameplay.
The 2026-09-26 Heroes III CEF-wrapper retest made Wine Steam readable under a
1,800 MiB guard, but the staged real game image still raised `Assert Failed`
before a playable scene; it remains unverified for gameplay.
The later original-Launcher Play test did create the separate game executable,
which exited with code 3 after duplicated handoff arguments. A no-duplicate
profile exists, but its first run was stopped for Steam CEF memory growth
before Play. A later bounded original-Launcher test removed the duplicate
arguments, but the game still exited with code 3; this disproves duplication
as the sole cause and is not a compatibility fix. A separate real cold-start
test confirmed Macroton's prelaunch resource guard sampled Steam/CEF before
handoff, then was replaced by one attributed guard; it did not test gameplay.
An SDL software-renderer A/B subsequently reached the real game through the
original Launcher but still exited with code 3 after four seconds; that
renderer switch is rejected as a fix.
A follow-up working-directory probe was interrupted by the prelaunch hard
memory guard at a 2,022.6 MiB sampled peak (2,000 MiB threshold); the guard
contained the exact prefix and cancelled the in-flight launch. The actual
Launcher cwd therefore remains unknown.
These classifications were rechecked with `macroton steam target-status` on
2026-09-26; a stored inventory/history record alone is not a current ownership
or gameplay check.
Braid's earlier native-viability conclusion was corrected on 2026-10-21: its
official Mac build is 32-bit and unusable on current macOS. It remains a
Windows fallback candidate, subject to the separate gameplay and Cloud gates.
Heroes III HD is selected first because the local Steam state already contains
the Windows payload and Macroton has an existing launcher-bypass path. The test
must still distinguish reaching the launcher/menu from actual 2D gameplay and
must not infer Steam Cloud success from local files alone.

The 2026-09-26 recheck had treated `native_status=working` as authoritative and
excluded Braid. That record was incorrect. The owner confirmed the native
32-bit limitation on 2026-10-21, consistent with the
[official Steam page](https://store.steampowered.com/app/26800/Braid/).
The eligibility file now says `legacy-unusable`; this resolves the conflict
with the older validation-matrix candidate row. It does not resolve Braid's
independent launch, Cloud, or gameplay blockers.
For Age II, the historical exploratory PE32 result is not substituted for a
fresh installed-payload check: the current isolated `steam install-status`
reports `not-requested`, and the exact prefix is idle with zero processes.
The [current official Age II page](https://store.steampowered.com/app/221380/Age_of_Empires_II_2013/)
labels the retired edition as 2D and lists Windows-only system requirements;
it does not prove current account entitlement or the bytes of a future depot.
After adding the content-addressed `eligibility/221380.conf` review, the
`steam inventory --2d-only` query admits Age II as `candidate-windows-only`,
while `--verified-win32-only --2d-only` still returns only Heroes III and
CHRONO TRIGGER. The isolated Age II prefix has no app-info cache, so a direct
`steam target-status` query against it fails before a payload gate; this is
not presented as a failed game launch or current PE check.

Static PE evidence for the installed `HOMM3 2.0.exe` is i386 with no detected
3D graphics import, SDL input, SDL/OpenAL/WinMM audio, and legacy DirectX plus
Visual C++ prerequisites. Silver's `silver.exe` is likewise i386 SDL/OpenGL,
but belongs to a 3D side lane. These engine/API classifications are useful routing signals, not a
substitute for visible gameplay evidence.

## Required evidence for the primary target

1. One clean single-instance Steam launch with the launcher bypass profile.
2. Visible title/menu and a campaign or skirmish board with 2D sprites/UI.
3. Keyboard and mouse interaction that changes the game state.
4. Audible music/effects during a board session.
5. Save, exit, reload and state restoration in the same isolated prefix.
6. Steam Cloud status/roundtrip evidence, or an explicit unsupported result.
7. A sustained session and clean exit without unbounded Wine memory growth.

Authentication is a prerequisite, not a compatibility result. Macroton keeps
each AppID in an isolated Wine Steam prefix; a stored local login record does
not prove that the Steam client has a live authenticated session. Test runs
must record the interactive-login state separately and must not promote a game
until a real game process and visible 2D surface are observed in that same
prefix.

3D frame-rate and Metal/DXVK tuning are deliberately not gates for this
matrix. Any 3D/renderer experiments remain separate diagnostic evidence.
