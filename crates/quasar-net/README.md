# quasar-net

Peer-to-peer input synchronisation for deterministic `quasar-sim` games:
lockstep and rollback sessions, UDP and in-memory transports, redundant
input packets, state-hash desync detection and confirmed-input journals.

```toml
quasar-net = { path = "../Quasar/crates/quasar-net" }
```

```sh
cargo run -p quasar-net --example net_duel
```

See [docs/networking.md](../../docs/networking.md).
