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

Starbyte currently executes **two narrowly bounded code forms**, only after the player explicitly enables them:

- **Single Game Genie codes** with Cheatarium's `game-genie` format: decode the cartridge CPU-bus address and replacement byte, then substitute that byte only when the guest CPU actually reads that cartridge address. No ROM file or cartridge image is rewritten. Enabling/disabling a cheat updates the live session; patches are not baked into save states, and are reapplied after restoring one.
- **Single eight-hex-digit WRAM address/value codes** targeting SNES banks 7E or 7F, as supported previously.

Unknown composition joins, multi-part Game Genie codes, unresolved encodings, and writes outside these supported classes remain visible **read-only**; Starbyte never silently applies the valid-looking portion of an unreviewed compound. A source label and syntactically decoded address are **not** ROM region/revision verification or proof the desired effect occurs. A code may be enabled yet have no effect if a different game build never reads the intercepted address. Testing currently covers synthetic cartridges and code reference vectors, **not confirmed effects in Super Mario World gameplay**. Use cheats with your own saves backed up.

Next work: checksum- and region-aware matching, reviewed compound-code execution, other code-device semantics and gameplay-specific cheat acceptance.
