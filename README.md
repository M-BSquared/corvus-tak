# TAK for Corvus GCS

A plugin for [Corvus GCS](https://github.com/M-BSquared/CorvusGCS), the
offline ground control station for PX4 and ArduPilot.

The contacts and markers of ATAK, CivTAK and WinTAK on the Home map, and the
aircraft on theirs: the local network, a TAK Server over TCP or TLS, the
callsign, and what is shared. The network side is part of Corvus GCS
(`src/core/src/tak`); this is the interface to it.

Install it in Corvus GCS under Settings, Plugins. The full description is in
the [Corvus guide](https://m-bsquared.github.io/CorvusGCS/guide/plugins.html#tak).

## What it is

A Rust crate (`corvus_plugin_tak`) built on `corvus_api`, the plugin API of Corvus GCS, and
published as a shared library from this repository.

- `src/model.rs`: the rules, free of the interface.
- `src/lib.rs`: the plugin, the map and what is shared.
- `src/view.rs`: what is drawn.
- `lang/`: German, French and the dialect file.

## Change it

Corvus GCS carries this repository as a submodule at `plugins/tak`, and the
crate is a member of its workspace:

```bash
git clone --recurse-submodules https://github.com/M-BSquared/CorvusGCS.git
cd CorvusGCS
./run.sh
```

Settings, Plugins then offers the copy built there. A dependency must be one
that `Cargo.lock` of Corvus GCS already holds.

## Publish it

Commit and push to `main`. The workflow tests the plugin, builds it for the
newest published Corvus GCS on Linux, macOS and Windows, lets that program load
it, and attaches it to the release `corvus-<version>` of this repository, where
Corvus GCS of that version finds it. A new version of Corvus GCS gets its build
within hours.

## Tests

From the Corvus checkout:

```bash
cargo test -p corvus_plugin_tak
```

## Licence

Sustainable Use License, see [LICENSE.md](LICENSE.md). Copyright (c) 2026
Maximilian Böck.
