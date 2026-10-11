# Portal windowless launch diagnosis — 2026-10-06

AppID 400 was reported as running by Steam while `hl2.exe` held one CPU core,
used about 30 MiB, and owned no WindowServer game window. A cold `+seh,+loaddll`
diagnostic showed the process loaded Wine's 32-bit system DLLs and then the
native Steam `gameoverlayrenderer.dll`; no Portal engine DLL or D3D9 DLL was
loaded. The same Portal executable hash and Macroton runtime fingerprint had
previously produced a visible game window, so a missing payload or wrong
runtime did not explain this regression.

Macroton already requested `gameoverlayrenderer=d;gameoverlayrenderer64=d` on
the later Steam `-applaunch` process. The resident Steam client creates the game
process, so that request did not prevent its overlay DLL from entering `hl2.exe`.
The client startup now receives the same narrow override. The existing login,
game files, and Cloud cache were retained; the stalled AppID 400 prefix was
stopped with `--preserve-session` before each diagnostic restart.

With the override on the resident client, Portal loaded `engine.dll`,
`portal/bin/client.dll`, and D3D9, then created a 1280×752 WindowServer game
window. A direct window capture showed the rendered test chamber. This verifies
visible presentation for this launch, not input, audio, save/reload, sustained
play, or the newest Cloud data on another device. Steam reported local Cloud
cache sync, while Macroton reported remote freshness as `unverified`.
