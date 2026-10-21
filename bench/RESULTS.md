# Results

One table over time. Every row: what ran, on what, wall time per phase, peak memory, output parity. Measured on
the user's Windows 11 machine (12 logical CPUs). "Load" notes what else was using the CPU, because Studio, the
Roblox player and other sessions' builds share the machine and move wall times by 2-3x.

Peak memory on Windows is the peak working set (`PeakWorkingSetSize`; Node's `process.resourceUsage().maxRSS` reports
the same counter, via `bench/maxrss.cjs`). Peak commit (private bytes) is given where known.

## Reference: the stock TypeScript 5 pipeline (the real project, 605 project files, 886 in the program)

| date | what | wall | peak WS | parity | load |
| --- | --- | --- | --- | --- | --- |
| 2026-10-21 | roblox-ts master + Flamework (TS 5.9), driver `--backend ts5` | 29.0 s (earlier, quiet machine); 86.2 s (busy) | 1474.6 MiB | reference (`exp/real-out-D-ts5`) | quiet / busy |
| 2026-10-21 | `tsc -p tsconfig.json --noEmit` (TS 5.9; also checks declaration files, 14 errors in `@rbxts` d.ts) | 20.5-20.9 s | 1028-1030 MiB | - | busy |

Earlier measurements (scratchpad `tsrs-roblox-ts/REPORT.md`): roblox-ts's own TS5 check of the project files ~6 s;
the tsrs API server check 1.0 s.

## M0: load + type-check in-process through tsrs (`rbxts-bench load`)

`rbxts-bench load [--checkers N] [--check project|all|none] [--lazy-dts]`, profile `measure` (release, no LTO), tsrs
v0.18.0 + fork branch `rbxts-fork`. Phases: config (tsconfig parse), program (`new_program`: read, parse, bind,
module resolution), check (what TypeScript's `getPreEmitDiagnostics` reports for the checked files: syntactic,
bind and semantic diagnostics with `@ts-ignore` / `@ts-expect-error` / `@ts-nocheck` applied, plus global
diagnostics). "project" = the 605 files roblox-ts compiles.

Three rounds of the five configurations, interleaved, one per row group (ms; peak working set and commit in MiB):

| date | checkers | checked | config | program | check | total | peak WS | peak commit | diagnostics | load |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-10-21 | 1 | project (605) | 18-22 | 124-140 | 1178-1306 | 1320-1468 | 134.5-135.0 | 266-276 | 0 | busy |
| 2026-10-21 | 4 | project (605) | 19-24 | 135-163 | 555-664 | 709-848 | 188.6-189.3 | 412-443 | 0 | busy |
| 2026-10-21 | 12 | project (605) | 16-25 | 118-154 | 404-565 | 557-744 | 245.0-246.0 | 586-639 | 0 | busy |
| 2026-10-21 | 1 | all (886) | 19-22 | 136-154 | 1924-2065 | 2094-2231 | 201.3-202.0 | 351-357 | 14 (the d.ts errors tsc and the tsrs CLI report) | busy |
| 2026-10-21 | 1, `--lazy-dts` | project (605) | 20-28 | 137-155 | 1265-1397 | 1423-1562 | 131.8-132.2 | 267-269 | 0 | busy |

Same-load comparison (separate interleaved runs earlier the same day, before `check` took on directives and global
diagnostics; the table above is after): ours 1.26-1.31 s total (1 checker) and
0.62-0.66 s (12 checkers) vs `tsc --noEmit` (TS 5.9) 20.5-20.9 s; the tsrs CLI `--noEmit --checkers 1` (all 886
files) 3.0 s wall, `--checkers 12` 1.0 s.

Diagnostics are checked against the tsrs CLI on small fixtures (`@ts-ignore`, used and unused `@ts-expect-error`,
`@ts-nocheck`, `skipLibCheck` on and off, a JSON import, `noLib` globals, unresolved references): the same
diagnostics. The one intended difference: with a syntax error the CLI stops after syntactic diagnostics, while
`check` (like TypeScript's `getPreEmitDiagnostics`) also reports semantic ones.

Findings:

- In-process tsrs matches the CLI: all files on one checker report the same 14 declaration-file errors; the 605
  project files check clean, as on TS5.
- Checking only the project files (what roblox-ts does; declaration files are only checked as far as the project
  uses them) takes ~1.2-1.3 s of checking on one checker, vs ~2.0 s for all files.
- More checkers: 12 checkers save ~0.7 s but cost ~110 MiB working set and ~330 MiB commit. One checker is the
  default for M1; per-checker transform is a later optimisation, not a requirement.
- `single_threaded` must not be used for "one checker": it also makes reading, parsing and binding serial (program
  phase 773 ms instead of ~140 ms). Set only `checkers`.
- `--lazy-dts` (declaration members parsed on first use) gives no clear gain here (within noise); leave it off.
- Memory: ~135 MiB peak for load + check vs ~1.0 GiB for TS5's tsc and ~1.5 GiB for the stock pipeline.
- The tsrs CLI's own `--extendedDiagnostics` "Check time" (1.1 s for 1 checker) undercounts its 3.0 s wall time;
  use wall time.

## M1 (part): Luau render parity (`rbxts-bench luau-parity`)

`rbxts_luau` renders the trees roblox-ts master (TS 5.9) builds, dumped by `tools/dump-luau-ast.cjs` at
`renderAST`, and the text is compared with roblox-ts's byte for byte. Render time is `rbxts_luau::render` over every
file (temporary-identifier solving included, JSON loading excluded), best of 20, profile `measure`. roblox-ts's
`renderAST` time is summed inside the same TS5 build that made the dump.

| date | corpus | files | identical | Luau | roblox-ts `renderAST` | `rbxts_luau`, 1 thread | 12 threads | load |
| --- | --- | --- | --- | --- | --- | --- | --- | --- |
| 2026-10-21 | the real project | 605 | 605 | 3.4 MiB | 421-472 ms | 23.5-25.1 ms | 5.6-5.7 ms | busy (CPU ~70%) |
| 2026-10-21 | roblox-ts tests (`tsconfig.bench.json`, without `src/diagnostics`) | 75 | 75 | 0.5 MiB | 103-138 ms | 5.1 ms | 1.6-1.9 ms | busy |
| 2026-10-21 | hand-written edge cases (`tools/luau-edge-cases.cjs`, committed fixture) | 12 | 12 | - | - | - | - | - |

The dump script also checks the TypeScript trees for what a parent-free renderer could get wrong: nodes whose
`parent` is missing or stale (13,133 / 6,316 of 377,527 nodes in the real project, mostly map fields and reused
identifiers), nodes shared between two places (60), and whether any parent-dependent decision (parentheses,
temporary-identifier scope edges, long-bracket spacing) would differ under the structural parent. None does, in
either corpus.

## M1 (part): project setup parity (`rbxts-bench project-parity`)

`rbxts_project`'s Rojo resolver, path translator and project setup against roblox-ts's own (`@roblox-ts/rojo-resolver`
1.3.0, `@roblox-ts/path-translator` 1.1.1, `src/Project` of roblox-ts master with the tsrs-experiment
`validateCompilerOptions`), dumped by `tools/dump-project-services.cjs`. Values: project data, path translator, Rojo
partitions / file mounts / warnings of the project's and the package resolvers, package mapping, project type,
runtime library path, `compileFiles`'s diagnostics, `resolve_module` probes, and per program file the output,
declaration, transformed, input and import paths with their Rojo paths, script types, network types and isolation.
Files are those in both programs: TypeScript 7 defaults `types` to `[]`, so `@rbxts/types` and friends that TS 5.9
pulls in through `typeRoots` are only in the JavaScript program unless imported.

| date | corpus | files compared | values | differences |
| --- | --- | --- | --- | --- |
| 2026-10-21 | the real project | 886 (of 956 in TS5's program) | 39,803 | 0 |
| 2026-10-21 | roblox-ts tests (root + its 2 project references, references with their graph-given options) | 305 | 10,844 | 0 |
| 2026-10-21 | Flamework monorepo `tests/place` (bun symlinks) | 105 (of 125) | 3,173 | 0 |
| 2026-10-21 | 12 hand-written setup fixtures (`crates/rbxts_bench/tests/setup_fixtures.rs`) | - | all | 0 |

Resolver build time, min / median of 5-10 (ms), profile `measure`. "shipped" is the published `RojoResolver.js`
(`fs.realpathSync`, JS implementation, twice per entry), "native" the same with `realpathSync.native`. "whole setup"
is `Project::setup`: rbxts options, project data, option checks, path translator, the project's resolver, one synthetic
resolver per type root and the package mapping (those three in parallel), and `compileFiles`'s checks.

| date | corpus | JS `fromPath`, shipped | JS `fromPath`, native | Rust `from_path` | Rust whole setup | load |
| --- | --- | --- | --- | --- | --- | --- |
| 2026-10-21 | the real project | 3318 / 3373 | 917 / 957 | 35 / 39 | 43-46 / 47-48 | busy (CPU ~55%) |
| 2026-10-21 | roblox-ts tests | 299 / 323 | 82 / 86 | 2.0 / 2.4 | 3.5 / 3.9 | busy |
| 2026-10-21 | Flamework `tests/place` | 654 / 672 | 122 / 136 | 6.4 / 7.8 | 16 / 17 | busy |

- The walk lists each directory once with the entries' own types (no `realpath`, no `stat` except for links) and
  spells out paths only for directories and project files.
- Partition checks (`checkRojoConfig`, lookups) compare already-resolved ASCII paths without building `path.relative`
  strings; that took the real project's setup from ~200 ms to ~80 ms before the walks were parallel.

## M1 (part): the T1 transform skeleton (`rbxts-bench build`)

`rbxts-bench build --optimizedLoops --allowCommentDirectives --write-partial`, profile `measure`, one checker:
load, project setup, check of the project files, transform in program order, render, write changed outputs. T1 ports
only part of the transform (DESIGN §9b): files using a kind not ported yet are reported and not written, so the
transform time covers less than the full work. Parity counts files whose output equals the reference byte for byte.
Three rounds interleaved with stock roblox-ts master (TS 5.9, `node out/CLI/cli.js build`, same flags; peak memory
from `bench/maxrss.cjs`) on 2026-10-21, machine moderately busy (ms; MiB).

| corpus | config | program | setup | check | transform | render | write | total | peak WS | identical | roblox-ts TS5 total, peak |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| the real project without Flamework (605 files) | 9-11 | 75-86 | 50-56 | 637-787 | 203-231 | 1.5-2.0 | 1.7-2.3 | 1019-1162 | 146.5-146.7 | 17 (588 unsupported) | 20.3-21.0 s, 611-618 |
| roblox-ts tests (`tsconfig.bench.json`, 72 files) | 2.3-2.5 | 39-49 | 3.8-4.7 | 85-103 | 75-81 | 0.8-1.2 | 1.2-2.2 | 216-236 | 62.5-62.8 | 17 (55 unsupported) | 6.5-7.2 s, 450-454 |

References: the tests project against the tsrs-backed roblox-ts build (`exp/out-C-tsrs`, TS 7 semantics), the real
project against `refs/real-noplugin-tsrs`. Every file that uses only T1 kinds is identical in both corpora, and so
are the 10 files of a hand-written T1 fixture (scratchpad `rbxts-rs/t1/fixture`) against the same tsrs-backed
pipeline.

## S2: incremental and watch rebuilds (`rbxts-bench watch-parity`, `rbxtsc build` with `"incremental": true`)

The real project without Flamework (`tsconfig.noplugin.json`, `--optimizedLoops --allowCommentDirectives`), on a
scratch copy, profile `measure`, one checker, 2026-10-21, machine busy (another session's builds, ms). A leaf is a
file nothing imports; the hub is imported by 76 files, and its closure (the reversed-import closure, DESIGN §14) is
205 of the 605 project files.

| build | wall | program | setup | invalidate | check | transform | render | write | working set |
| --- | --- | --- | --- | --- | --- | --- | --- | --- | --- |
| watch: initial (clean) build | 3229 | 488 | 108 | 14 | 883 | 684 | 28 | 1003 | 179 MiB |
| watch: leaf edit (median of 41) | 66 | 0 | 0 | 25 | 1 | 0 | 0 | 25 | +1 MiB each |
| watch: hub edit (median of 11) | 874 | 21 | 0 | 23 | 419 | 304 | 15 | 40 | +48 MiB each |
| `rbxtsc build`, incremental, nothing changed (new process) | 278-294 | | | | | | | | |
| `rbxtsc build`, incremental, leaf edit | 259-261 | | | | | | | | |
| `rbxtsc build`, incremental, hub edit | 969-1004 | | | | | | | | |
| `rbxtsc build`, clean (no incremental state) | 1428-2143 | | | | | | | | |

- A watch rebuild of edited files updates the program in place (tsrs `reuse_program`) and keeps the project setup;
  "invalidate" is the file facts of all 886 program files, the closure and the listing of the output directory;
  "write" is the Luau, the asset sync and the clean of stale outputs.
- Programs are never freed (tsrs's `free_program` is `unsafe`), so the watcher's working set grows by the checker
  of each rebuild: about 1 MiB per leaf edit and 48 MiB per hub edit here. `rbxtsc -w` restarts its process past
  `RBXTS_WATCH_MEMORY_LIMIT_MB` (default 2048).
- Stock roblox-ts takes about 29 s (quiet machine) for any build of this project.
