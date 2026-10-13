# Starbyte and local Cheatarium

Starbyte uses a player-configured, local Cheatarium checkout for SNES cheat source candidates. The Starbyte runtime configuration lives at `$XDG_CONFIG_HOME/starbyte/config.toml` (usually `~/.config/starbyte/config.toml`), and accepts:

```toml
[cheats]
cheatarium_root = "~/path/to/cheatarium"
```

The native GUI exposes this as **Settings → Cheats → Local Cheatarium checkout**. Save a folder (or Browse), and Starbyte refreshes the library's cached cheat candidates from the local `generated/v1/snes.json.gz` when that index exists. It never downloads Cheatarium cheats or copies the collection into the Starbyte source tree. Other legacy Libretro provider behavior remains available only when no local Cheatarium checkout is configured.

The read-only CLI also uses the configured local index without requiring a path:

```sh
cargo run -p starbyte-cli -- cheatarium --title "Super Mario World" --json
```

To override this for an individual lookup, provide `--index /path/to/snes.json.gz`. The CLI command always performs candidate title search only and **never enables a cheat**.

The desktop library's **Refresh Cheats** action now reads the same configured local index. Titles are matched conservatively against game/library names after removing parenthesized filename suffixes. Candidate matches are not confirmed by ROM hash, revision, region, or observed gameplay. Each indexed code preserves its original source text, source ID, description and format label in Starbyte's cache. Original Cheatarium data is never rewritten.

Only an individually decoded eight-hex-digit SNES work-RAM address/value code targeting banks 7E or 7F is currently offered as an executable local candidate. Its checkbox is explicitly user-controlled. Source codes in other formats, including **Game Genie ROM-read substitutions**, uncertain compositions and codes for other address spaces are displayed read-only rather than deceptively enabled. Even executable syntax **does not** establish that the code is compatible with the user's ROM, so enable candidates at your own discretion and keep saves backed up.

Next work: checksum- and region-aware matching, authentic Game Genie read substitution, other code-device execution models and gameplay-specific cheat acceptance. The distinction between a title candidate, a syntactically decoded code, and a verified working cheat must remain explicit.
