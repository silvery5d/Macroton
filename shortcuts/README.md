# Game shortcuts

`catalog.tsv` is the portable list of game names used for generated Finder
shortcuts. Add one tab-separated `AppID<TAB>Display name` row for a game with a
reviewed `compatibility/APPID.conf` profile. Use the game's recognizable public
name; do not add local paths, account data, Steam tickets, or game files.

Run `macroton shortcut APPID` after building Macroton. It creates
`shortcuts/Display name.command` locally. Generated `.command` files contain
the builder's absolute binary and profile paths, so they are machine-specific
outputs and must not be committed. Another developer can regenerate them from
the same catalog and profile.

An entry names a shortcut only. It does not claim that the account owns the
game or that gameplay has passed compatibility review.
