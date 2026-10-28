# Shared configuration and CLI

Issue #42 is implemented by the independently usable `quasar-config` crate.
It provides typed settings, provenance, precedence, validation, TOML/JSON file
loading, explicit environment mappings, common CLI flags and versioned
migrations without a global configuration or custom build system.

## Layers and precedence

```text
defaults < TOML/JSON file < environment < common CLI/game CLI < --set < programmatic
```

`Config` is an ordered dotted-key map with a `Source` per value. Lower-priority
writes are ignored, and `provenance(key)` reports which layer won. Game-defined
settings are ordinary typed reads; the crate does not dictate a schema.

## Typed settings and validation

`get_bool`, `get_u64`, `get_i64`, `get_f32`, `get_f64`, `get_string`,
`get_resolution` and `get_or` return structured `ConfigError` values with the
key and source. Helpers such as `validate_u64_range`, `validate_f64_range` and
`validate_non_empty` give games reusable validation primitives.

`Config::migrate(target, migrations)` applies consecutive `from -> to` steps,
preserves unknown keys, updates the schema version and rejects a newer schema
instead of silently downgrading it.

## Files and paths

TOML is the recommended human-edited format (comments, nested tables and typed
Serde values); JSON is fully supported for compatibility and machine-generated
files. `FileFormat::from_path` selects `.toml` or `.json` explicitly; unknown
extensions are errors, never guessed. Loading never overwrites or converts an
existing file. The optional `storage` feature loads a shared Basalt config
through `GamePaths`.

`load_file_typed::<T>` provides strongly typed Serde loading. `TypedConfig::<T>`
loads through the precedence-aware map and deserializes the merged scalar
values. Complex arrays should use the direct typed file API.

## Common CLI and environment

`CommonArgs` handles `--headless`, `--fullscreen`, `--windowed`,
`--resolution WxH`, `--no-audio`, `--audio-device`, `--benchmark`,
`--benchmark-file`, `--log-level`, `--config`, repeatable `--set`, `--help` and
`--version`. Unknown flags and positional values are returned untouched so a game
adds its own flags without conflicts. `CommonArgs::from_env(prefix)` reads the
matching `PREFIX_*` variables; game config keys use explicit mappings through
`Config::apply_env[_with]`.

`crates/quasar-config/examples/desktop.rs` is a MissileCommand-like desktop
application and `examples/server.rs` is a MattMC-like headless server. They share
the primitives but define distinct player/server flags.

`examples/starfall` demonstrates the same APIs in a real desktop/headless game:
its default `config.toml` (with comments), optional JSON compatibility,
environment overrides, `--resolution`, `--set`, controls, recording, replay,
screenshots and benchmarks. Existing JSON files are never converted.

## Acceptance criteria

- Independent Cargo applications with shared common arguments and distinct game
  flags, with normal `cargo run -- --help`.
- Typed defaults/file/env/CLI precedence and provenance.
- Resolution parsing, validation, conflict/error reporting and env overrides.
- Non-destructive malformed-file handling and isolated config paths.
- Consecutive migration with unknown-key preservation and newer-schema rejection.
- Optional Basalt/storage adapter and no mandatory TOML/Clap/Qt/ECS/runner.
- Headless server and GUI-style examples.
- No centralized mutable global configuration.

## Verification

```sh
cargo test -p quasar-config
cargo test -p quasar-config --no-default-features
cargo test -p quasar-config --features storage
cargo run -p quasar-config --example desktop -- --help
cargo run -p quasar-config --example server -- --help
cargo clippy -p quasar-config --all-targets -- -D warnings
cargo run -p starfall -- --help
```

## Limitations

- TOML and JSON are the built-in persisted formats.
- The map-based API stores arrays as serialized text; typed file APIs handle
  complex arrays/tables.
- The common parser is intentionally not a full subcommand framework.
- Environment parsing is heuristic and typed getters remain authoritative.
- Unknown file keys are preserved in memory but not written back automatically.
- The server example currently installs its game-specific `--bind`,
  `--max-players` and `--tick-rate` arguments as defaults before loading a file.
  File values therefore override those explicit arguments. Consumers should
  install defaults, load files, then apply explicit game CLI values with
  `set_cli`, followed by `CommonArgs::apply_overrides`. The example still needs
  that correction; the library's precedence handling is already available.
