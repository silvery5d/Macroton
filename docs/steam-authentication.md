# Macroton Steam authentication runbook

Macroton deliberately keeps account-owner interaction in the visible Windows
Steam UI. It never accepts, stores, or types a password. A QR scan or a line in
Steam's connection log is only an observation; it is not proof that the next
game launch will reuse the session.

## Recommended flow

Use one persistent AppID prefix and the same profile for all three commands:

```sh
target/debug/macroton steam login 105450 \
  --profile compatibility/experiments/105450-shadowalias-vulkan.conf \
  --data-root "$HOME/Library/Application Support/Macroton" \
  --max-rss-mib 1000
```

Complete the login or confirmation in the visible Wine Steam window. Then
inspect the bounded verification result:

```sh
target/debug/macroton steam login-wait 105450 \
  --data-root "$HOME/Library/Application Support/Macroton"
target/debug/macroton steam prefix-status 105450 --json
target/debug/macroton steam launch-status 105450 --json
```

`login-wait` may report persisted signals after Steam exits. That means the
prefix contains stored login metadata, userdata, a connected account context,
and a recent logged-in UI state. It does **not** guarantee that a fresh Steam
process will render the post-login UI or accept `-applaunch`.

The launch boundary is the authoritative check. If the next launch shows
`Sign in to Steam`, treat the session as stale/inconsistent, do not start a
game or send input, and repeat the visible `steam login` flow in the same
prefix. If `launch-status` reports `waiting-user`, complete the visible
interstitial while that exact-prefix Steam client is still live; if it reports
the event is stale, retry the launch.

When the bounded login command fails, its error now includes redacted
`transport`, `authenticated-context`, and `ui-login` fields. For example,
`transport=disconnected` points to Steam CM/network convergence, while
`transport=connected` with `ui-login=false` points to a Wine/CEF presentation
failure. These fields are diagnostics only and never substitute for a visible
authenticated launch.

## Evidence rule

Only claim “Steam launch authenticated” after the exact prefix has both a live
primary Steam client and a tracked game process (or a visible post-login UI
whose pixels and transport are independently confirmed). Stored-login files,
historical `state 5` entries, a QR page, or a successful `login-wait` by itself
must never be used as gameplay, Cloud, or ownership proof.

## Recovery

If a bounded attempt leaves Wine processes behind, stop only the AppID prefix
with Macroton's stop command, then retry. Do not delete `userdata`, `config`,
or Cloud files to “fix” authentication; doing so destroys the evidence needed
to distinguish a stale session from a real Steam or Wine failure.
