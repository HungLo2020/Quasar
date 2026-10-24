# quasar-config

Reusable typed settings and common CLI arguments for Quasar games and ordinary
Rust applications. There is no global configuration and no file is ever
overwritten.

```toml
quasar-config = { path = "../Quasar/crates/quasar-config", features = ["toml", "json"] }
```

TOML is the recommended default for human-edited files; JSON remains supported
for compatibility and machine-generated data. Format is selected by extension
(`.toml` or `.json`), never guessed and never rewritten.

```rust
use quasar_config::{CommonArgs, Config};

let mut common = CommonArgs::from_env("MYGAME")?;
common.parse_into(std::env::args().skip(1))?;
if common.help {
    print!("{}", common.help("mygame", "Game options:\n  --difficulty EASY|HARD"));
    return Ok(());
}
let mut config = Config::new();
config.set_default("window.width", 1280_u32)?;
config.set_default("window.height", 720_u32)?;
config.load_file("game.toml")?; // or game.json; extension selects parser
common.apply_overrides(&mut config)?;
let width = config.get_u64("window.width")?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

## Precedence

`default < file < environment < CLI < programmatic`

`Config` records provenance per key. Lower-precedence assignments do not
overwrite higher ones. CLI `--set KEY=VALUE` is applied last.

## Features

- `toml` (default): comments, nested tables and typed Serde loading.
- `json` (default): flattened JSON object files and JSON compatibility. Arrays
  are retained as serialized text; unknown keys are preserved.
- `storage`: optional Basalt `GamePaths` shared-config loading.
- `load_file`/`load_file_typed`/`TypedConfig` select format by file extension.
- No Clap dependency. `CommonArgs` is a small, composable parser; game-defined
  flags remain in `remaining`.
- Core modeling, env mapping, CLI parsing and migration work with
  `--no-default-features`.

## Common options

`--headless`, `--fullscreen`, `--windowed`, `--resolution WxH`, `--no-audio`,
`--audio-device`, `--benchmark`, `--benchmark-file`, `--log-level`, `--config`,
`--set`, `--help`, `--version`.

Environment equivalents use a caller prefix, for example `MYGAME_HEADLESS`,
`MYGAME_RESOLUTION`, `MYGAME_CONFIG`. Game-defined mappings are explicit key →
variable pairs, avoiding accidental global env reads.

## Typed Serde APIs

`load_file_typed::<T>` parses a `.toml` or `.json` file directly with Serde.
`TypedConfig::<T>::load_file` loads through `Config` and deserializes the
precedence-resolved scalar map. `Config::deserialize::<T>()` is available with
JSON serde support; use typed direct file loading for arrays/tables when needed.

## Failure behavior

Missing files are optional; malformed existing files, invalid values, unknown
explicit CLI keys and newer schemas return structured errors. Parsing never
writes or deletes a user config. `migrate` runs consecutive version steps and
preserves unknown fields; a newer schema is rejected rather than downgraded.

## Verification

```sh
cargo test -p quasar-config
cargo test -p quasar-config --no-default-features
cargo test -p quasar-config --features storage
cargo run -p quasar-config --example desktop -- --help
cargo run -p quasar-config --example server -- --help
cargo clippy -p quasar-config --all-targets -- -D warnings
```

## Limitations

- TOML and JSON are the built-in persisted formats; other formats can use
  `Config::set_*` or game-defined adapters.
- The map-based `Config` stores arrays as serialized text; use the typed file
  APIs for complex nested arrays/tables.
- The common parser is intentionally small, not a general subcommand framework.
- Environment values are string-parsed heuristically; typed getters reject
  incompatible values.
- Unknown keys are retained in memory but not automatically written back.
