# Contributing a game configuration

Macroton accepts one compatibility profile per Steam AppID in
`compatibility/APPID.conf`. The configuration format and parser are MIT licensed.
You can submit a profile even when the game is not yet fully playable; describe
the observed state clearly in the pull request. A profile is a launch candidate,
not a compatibility certification.

1. Check that the game has a legitimate Steam AppID and that its native macOS
   build does not already work on the target Mac. Add or update
   `eligibility/APPID.conf` with the official store URL, reviewed date, observed
   native status, scope, and a concise evidence code. Use an existing file as
   a format example; do not copy its conclusion to another game. Do not submit game files,
   Steam credentials, tickets, private account data, runtime binaries, or logs
   containing any of these.
   An optional `display_name` gives a reviewed local-language title in the
   Macroton library; it changes neither Steam metadata nor compatibility.
2. Copy `compatibility/123456.example.conf` to `compatibility/APPID.conf` and
   replace the values with facts observed for that game. The filename and
   `appid=` must match. Use `schema=1`.
3. Run `cargo run -- profile validate compatibility/APPID.conf`,
   `cargo run -- profile validate compatibility/APPID.conf --appid APPID`,
   `cargo run -- eligibility validate eligibility/APPID.conf`, and
   `cargo test --all-targets`. Validation checks syntax and safety constraints;
   it does not prove the native build is unusable, the game launches, or Steam
   Cloud is correct.
4. In the pull request, provide the Steam store URL, macOS version and chip,
   game build/depot identifiers if known, runtime and renderer tried, and a
   concise account of what actually happened: installation, launch, visible
   frame, input, audio, save/reload, Cloud sync, and duration. Mark untested
   items as untested. Include reproduction steps and sanitized diagnostics
   when a gate fails. Do not claim full support from a successful process exit.
5. Add `AppID<TAB>Game name` to `shortcuts/catalog.tsv` only when the name is
   verified. This row names a shortcut and makes no playability claim.

## Format

Each nonempty line is `key=value`; `#` starts a full-line comment. Keys may
appear once, except `env.NAME` uses a distinct environment variable name.
The parser rejects unknown keys and launcher-controlled environment variables.

Required keys are `schema`, `appid`, `runtime`, `runtime_version`, `renderer`,
and `executable`. `executable` is relative to the selected game payload.
`launch=steam` additionally requires `steam_install_dir`. The default launch
mode is direct. The default payload root is `native-depot-content`; use
`windows_payload_root=steam-installed-tree` only for a Windows Steam install.

For one Windows depot, use `windows_depot_id` and optionally
`windows_depot_manifest_id`. For several, use `windows_depots` as comma-separated
`DEPOT_ID:MANIFEST_ID` entries in overwrite order. Do not combine the two forms.
Optional `windows_build_id` pins the observed build. Leave identifiers absent
when unknown rather than inventing them.

`graphics_api` can be `unknown`, `d3d9`, `d3d10`, `d3d11`, or `opengl`.
`renderer` accepts `auto`, `wine-default`, `wined3d-opengl`,
`wined3d-opengl-serialized`, `wined3d-vulkan`,
`wined3d-vulkan-serialized`, `dxvk-d3d9-moltenvk`, or `dxmt`.
These are parser values, not guarantees that a runtime is installed.

For Steam Cloud, `cloud_files` is a semicolon-separated list of
`REMOTE_PATH|LOCAL_PATH` mappings. Local paths default to the Wine user
directory; `user:`, `steam-install:`, and `steam-remote:` select explicit roots.
Use `steam_cloud=required` only with verified mappings. To record lack of Steam
Cloud support, use `steam_cloud=unsupported` with an official Steam store URL
in `steam_cloud_source` and a `YYYY-MM-DD` `steam_cloud_reviewed_date`.
Otherwise leave the policy unspecified. Do not include actual save files.

Profiles are reviewed for provenance, safe paths, and accurate claims before
merging. A merged profile can still be experimental. Full acceptance requires
the separate automated evidence and human gameplay review described in
`docs/testing.md`.
