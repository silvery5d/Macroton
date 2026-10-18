## Game configuration

- Steam AppID and store URL:
- Files changed: `compatibility/APPID.conf`, `eligibility/APPID.conf` (and optional shortcut name)
- Native macOS status and how checked:
- macOS version and Mac chip used for testing:
- Game build/depot identifiers observed, or unknown:
- Runtime, version, renderer, and graphics API tried:

## Observed behavior

Mark each item passed, failed, or untested. Include concise reproduction steps
for failures; redact account names, credentials, tickets, save data, and paths
containing personal information.

| Gate | Result and evidence |
| --- | --- |
| Steam installation and ownership prompt | |
| Game process and visible frame | |
| Input and audio | |
| Save and reload | |
| Steam Cloud sync | |
| Sustained play and clean exit | |

## Local checks

- [ ] `cargo run -- profile validate compatibility/APPID.conf`
- [ ] `cargo run -- eligibility validate eligibility/APPID.conf`
- [ ] `MACROTON_STEAM_HANDOFF_RETRY_SECONDS=1 cargo test --all-targets --quiet`

The configuration alone does not establish full compatibility. State any
unverified behavior explicitly.
