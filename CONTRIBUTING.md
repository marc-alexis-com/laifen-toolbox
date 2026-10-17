# Contributing

Thanks for helping. This tool was worked out on one brush; it grows when people with other
brushes, firmwares or shells share what theirs does.

## Report your brush

The quickest, most useful thing is a capture from your own brush. Open a
[brush report](https://github.com/marc-alexis-com/laifen-toolbox/issues/new?template=add-a-brush.yml)
with:

- `laifen scan` — the name it shows up as;
- `laifen status --json` — every setting the tool reads;
- `laifen watch --raw`, then press the button and cycle the modes — the frames the brush sends on
  its own.

Mask the last part of the address and any serial number before you post; the four hex digits of
the name suffix are enough to see the pattern.

Everything above only reads the brush, or changes settings the brush reports back; none of it is
destructive. Please don't paste output from commands the tool refuses
([docs/PROTOCOL.md](docs/PROTOCOL.md) §6) — they were left out on purpose.

## What is and isn't supported

`laifen` speaks the protocol of the Wave (`LFTB01-P`, and the stainless and aluminium shells that
share it). The SE family (`LFTB01-SE`), the TB92 (`LFTB02-S`) and the TC51 kids' brush
(`LFTB01-C`) speak a different, longer protocol, read from the vendor app but never run against a
device (see the appendix of [docs/PROTOCOL.md](docs/PROTOCOL.md)). A capture from one of those is
still welcome: it is how support for them would start.

## Changing the code or the protocol notes

- `cargo test`, `cargo clippy --all-targets -- -D warnings` and `cargo fmt --check` all pass.
- In `docs/PROTOCOL.md`, keep each fact marked **confirmed** (seen on a device), **app** (from the
  vendor app) or **inferred**, and say which brush and firmware it was seen on.
- The tool only sends commands confirmed on a real brush, and reads every write back. A change
  that sends something new should say how it was verified, and on what hardware.

By contributing you agree your contribution is licensed under the repository's MIT license.
