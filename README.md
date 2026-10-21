# rbxts-rs

roblox-ts rewritten in Rust on [tsrs](https://github.com/oxc-project/tsrs) (the Rust port of TypeScript 7), with
Flamework as an in-process plugin. Private and local: never pushed or published.

- Design: `E:\Projects\TS\Flamework\scratchpad\batch\rbxts-rs\DESIGN.md`.
- Measurements: `bench/RESULTS.md`.
- tsrs: a fork branch `rbxts-fork` in `E:\Projects\TS\Flamework\worktrees\tsrs-rbxts-fork`, used through path
  dependencies (see `Cargo.toml`).

## Build and measure

```sh
cargo build --profile measure
cd <project with tsconfig.json>
<rbxts-rs>/target/measure/rbxts-bench.exe load [--checkers N] [--check project|all|none] [--lazy-dts] [--json]
```

`dev` builds are optimised too (a debug checker is unusably slow). Use `measure` for any number that goes into
`bench/RESULTS.md`, and interleave runs with the reference because machine load moves wall times.
