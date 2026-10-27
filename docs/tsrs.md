# tsrs: how rbxts-rs depends on it, and how to update it

rbxts-rs links tsrs, the Rust port of TypeScript 7, directly as Rust crates in the same process. There is no wrapper,
no IPC and no patched tsrs code. Everything roblox-ts-specific lives in rbxts-rs. That includes the places where we
emulate TypeScript 5 behaviour that TypeScript 7 changed.

## The fork
- **Where:** a worktree of the tsrs clone at `E:\Projects\TS\Flamework\worktrees\tsrs-rbxts-fork`, on branch
  `rbxts-fork`. `Cargo.toml` points at its crates by absolute path, so every rbxts-rs worktree uses the same tsrs.
- **What:** tsrs **v0.18.0** (`5f92808`) plus commits that change only visibility, `pub(crate)` → `pub`. That is
  10 lines in 8 files:

| Commit | Item | File | Used for |
|---|---|---|---|
| 63b22eb | `Checker::create_array_type` | `tsrs_checker/src/checker_12.rs` | the transform's type queries |
| 63b22eb | `Checker::get_string_literal_type` | `tsrs_checker/src/checker_12.rs` | the transform's type queries |
| 63b22eb | `Checker::get_number_literal_type` | `tsrs_checker/src/checker_12.rs` | the transform's type queries |
| 63b22eb | `Checker::get_type_of_assignment_pattern` | `tsrs_checker/src/services.rs` | destructuring assignments |
| cf8498b | `Program::get_semantic_diagnostics_with_checker` | `tsrs_compiler/src/program.rs` | checking that honours `@ts-ignore` and the other comment directives |
| adadb25 | `Checker::get_contextual_signature` | `tsrs_checker/src/checker_05.rs` | nothing since TypeScript 5's context sensitivity is marked on the tree (below); kept, as the fork is shared |
| 87958da | `Checker::is_readonly_array_type` | `tsrs_checker/src/checker_11.rs` | Flamework's macro types (`flamework_rs`) |
| 87958da | `Checker::get_literal_type_from_property` | `tsrs_checker/src/checker_13.rs` | Flamework's macros (`flamework_rs`) |
| 87958da | `is_declaration_readonly` | `tsrs_checker/src/utilities.rs`, `tsrs_checker/src/lib.rs` | Flamework's macro types (`flamework_rs`) |

  The branch also carries 67b5330 (overload internals made public) and its revert c796de98, which cancel out.

- **Rule:** the fork gets visibility flips and nothing else. Behaviour fixes and workarounds go in rbxts-rs with a
  `// rbxts-rs:` comment. If a needed function is too entangled to make public, copy a small helper into rbxts-rs
  with a note instead.
- **Upstreaming:** these flips are reasonable public-API requests to tsrs. Each one upstream accepts is one less to
  carry.

## Updating tsrs
Pin the version and update on purpose, never automatically. tsrs is pre-1.0, so a release can change its API.

1. **Rebase the flips.** Fetch the new release in the tsrs clone. Make a new branch from the release tag and
   cherry-pick the three commits above. If upstream has made an item public already, drop that flip.
   - Keep the old `rbxts-fork` branch until the update is merged, so you can roll back.
2. **Update `Cargo.lock`.** It started as a copy of tsrs's lock file, so the transitive versions match what tsrs tests
   with. Re-sync the shared dependencies with the new tsrs lock file.
3. **Fix API breaks.** Build the workspace and fix every compile error: renamed or reshaped checker methods, AST
   accessors, flags and symbol lookups. This is mechanical, and the compiler finds all of it.
4. **Catch behaviour changes.** These are the subtle part, because they compile cleanly but change the output. Run
   parity on both corpora and compare with the baselines:
   - the roblox-ts tests project, 72 files;
   - the real project with Flamework off, 605 files.

   Use `merge\parity.sh` and `merge\parity-{tests,real}-main.txt` in the scratchpad's `batch\rbxts-rs` folder, plus
   `cargo test --workspace` for the fixtures and plugin tests. Some files differ from stock on purpose, where
   rbxts-rs fixes a roblox-ts bug. `rbxts-bench build --parity REF --divergences crates/rbxts_bench/parity/divergences.json
   --corpus {roblox-ts-tests,real-noplugin}` reports those as `known-fix` while their output is unchanged (see
   `crates/rbxts_bench/src/divergences.rs`). Any other differing file, or a `stale` one, is a behaviour change. Find
   the tsrs commit that caused it, then either adapt rbxts-rs (with a `// rbxts-rs:` note) or report it upstream.
5. **Re-measure** `rbxts-bench load` and a full build (see `bench/RESULTS.md`), so a performance regression doesn't
   slip in.
6. **Process:** as with every change, a worker does the update and a fresh validator re-runs parity before it merges.

## TypeScript 7 behaviour the port depends on
Re-check these after an update; they are where tsrs's behaviour reaches our output.
- `getTrueType()` without an argument is the *regular* true type (`regular_true_type`), not the fresh one. `??`
  lowering depends on it.
- `get_exports_of_module` keeps the symbol table's insertion order, which is TypeScript 5's: the binder binds a
  block's function declarations first (`bind_each_statement_functions_first`), and `export *` adds its names after the
  module's own. rbxts-rs uses that order for export tables and namespaces (`TransformState::get_module_exports`);
  TypeScript 7's API sorts the exports with `compare_symbols_exported`, which is not used.
- An omitted binding element (`[, a]`) is a nameless `BindingElement`.
- `Node::text()` panics on a `QualifiedName`.
- Positions are UTF-8 byte offsets. `tsrs_ast::positionmap` converts them to UTF-16 where text positions reach the
  output.
- **Context-sensitive functions.** TypeScript 5 takes every function expression and object literal method without
  type parameters and without a `this` parameter for context sensitive (its implicit `this` is contextually typed);
  TypeScript 7's `has_context_sensitive_parameters` takes only one that uses `this` (its `ContainsThis` flag).
  That decides whether such a function gets its contextual `this` (`function () {}` in a `(this: Foo) => void` slot
  is `function(self)` in stock's output), and in an overloaded call it decides which overload fixes that `this`
  (TypeScript 5 fixes it from the first overload it checks the function against in full, even one it then rejects),
  generic inference, and when no overload matches (TS2769). `rbxts_project` (`mark_ts5_context_sensitive_functions`)
  sets `ContainsThis` on those functions after binding, before anything is checked, so TypeScript 7's checker judges
  them as TypeScript 5's; watch marks each file it binds again. Nothing else reads the flag on a function node:
  re-check that (`ContainsThis` in `tsrs_checker` and `tsrs_ast`) after an update.
- `implements` entries are TypeReference nodes. `getIndexTypeOfType` takes a key type. A property access through an
  index signature gives an `__index` symbol. There is no `__String`.
- TypeScript 7 gives the related information of `TS1007` ("The parser expected to find a '}' to match the '{' token
  here.") a length of 0; rbxtsc prints it with TypeScript 5's length of 1 (`rbxts_cli/src/pretty.rs`), so its
  underline matches stock's.
- **Automatic type directives.** Without `types` in the tsconfig, TypeScript 5 includes every package of the type
  roots in the program; TypeScript 7 (tsrs `get_automatic_type_directive_names`) includes none (only `"types": ["*"]`
  lists the type roots, by TypeScript 5's rule, with TypeScript 5's wording where the two differ: TS2688's "Entry
  point for implicit type library", "Cannot find name 'process'" without "add node to the types field"). rbxts-rs
  restores TypeScript 5's rule: `rbxts_project` puts `"types": ["*"]` into the parsed options when the tsconfig has
  no `types` (`add_automatic_type_directives`; roblox-ts's own check of `types` still sees the tsconfig's), so a
  project, or a referenced project, whose sources import no package still sees `@rbxts/compiler-types` (TypeScript 7
  alone stops at `TS2318: Cannot find global type 'Array'`). The type roots are listed through the program's file
  system, so watch and incremental builds see a package appear, change or go. On the real project that loads 70 more declaration files, from 8
  packages nothing imports, as TypeScript 5 does.
- **`baseUrl`.** TypeScript 7 removed it (`TS5102`, and an import that only resolves through it is `TS2307`).
  `rbxts_project` emulates TypeScript 5 (`emulate_base_url`): the `paths` substitutions are made absolute against
  `baseUrl`, `"*": ["<baseUrl>/*"]` takes the names no pattern matches (TypeScript 5 tries `baseUrl` only for those: a
  matched pattern that resolves nothing is TS2307 in both), and `baseUrl` is cleared.
- **`rootDirs` without `rootDir`.** With an output directory and no `rootDir`, TypeScript 5 lays the outputs out under
  the common source directory of the emitted files; TypeScript 7 under the tsconfig's directory, refusing a project
  whose files have another one (`TS5011`). `rbxts_project` gives the program TypeScript 5's common source directory
  as its `rootDir` when it is not the tsconfig's (`ts5_root_dir`; from the root files, and the program is made again
  in the rare case that its files reach further), and roblox-ts and plugins see the tsconfig's options
  (`Project::options`).
- **Threading:** the checker is not `Send`, and `get_type_checker` takes a non-reentrant guard. Checking needs a
  512 MiB stack. Set only `checkers`: `single_threaded` also makes parsing and binding serial.

## TypeScript 5 behaviour rbxts-rs does not reproduce
Known, accepted differences from stock roblox-ts on TypeScript 5, each inherent to TypeScript 7. A few make a build
fail where stock's passes, and the entry says how to write it for both; one changes only a diagnostic's text. The
last section lists Luau engine bugs that stock's output, and so rbxts-rs's, triggers.

- **Union and mapped key order.** Where an output lists the properties of a union or a mapped type, TypeScript 7's
  type ids order them, not TypeScript 5's (`util/ts5_property_order.rs` restores TypeScript 5's order where it can be
  derived from the program). It cannot for a union whose members have the very same symbols (`PXY | PYX`: the first
  member's order is used), a mapped type whose `as` clause makes several names from one key (`` `get${K}` |
  `set${K}` ``: TypeScript 7's order), and guards and the plugin facade's `getFields`, which use the raw order (as
  TypeScript 5's `Record`/`Pick` key order follows its own type ids). Flamework sorts where the order reaches its
  output.

- **Declaration-emit symlinks.** With `declaration: true`, TypeScript 5's pre-emit diagnostics run the declaration
  emitter, and when it needs an `import("…")` specifier for a type from another module, its module-specifier code
  (`getAllModulePathsWorker`) resolves every runtime dependency (`dependencies`, `peerDependencies`,
  `optionalDependencies`) of the importing file's `package.json` and adds their links to the program's symlink cache.
  roblox-ts's `guessVirtualPath` reads that cache, so from then on (in transform order) an import of a package's real
  path, a relative import or a `paths` alias into a sibling package that is also a linked dependency, goes through
  `node_modules` (`include/node_modules/@scope/pkg`). rbxts-rs's symlink map (`util/symlink_cache.rs`) is built from
  the program's resolutions only, as TypeScript 5's cache starts out, so such an import keeps the package's own path
  (`packages/pkg`). Not modelled: rbxts-rs runs tsrs's declaration diagnostics and emit, but tsrs's specifier
  lookups do not reach rbxts-rs's symlink map, and reproducing when the emitter asks for a specifier would mean
  tracking it for every file. To get the `node_modules` path in both, import
  the package by its name. (Probe: a relative import of a sibling `file:` dependency, with a declaration build.)
- **Grammar errors TypeScript 7 added.** TypeScript 7's checker reports some grammar errors TypeScript 5.9 does not,
  for example `error TS18064: Private identifiers cannot be used in destructuring patterns.` (`const { #x: v } = obj`).
  A file with one fails to build in rbxts-rs, where stock roblox-ts reports its own diagnostic for the construct when
  it reaches the file (or nothing, when an earlier file's error stops it). Both refuse the code; only the message
  differs.
- **Declaration maps.** TypeScript 7 maps some declarations (constructors, a specifier `transformPaths` rewrites)
  differently from TypeScript 5: `.d.ts.map` files are not byte-equal to stock roblox-ts's. The `.d.ts` files are.
- **TS2769's elaboration.** When no overload matches a call and two or three were tried, TypeScript 5 elaborates the
  error of each (`Overload 1 of 2, '(cb: (this: Foo) => number): void', gave the following error.` and the same for
  overload 2, each with its own chain); TypeScript 7 (`report_call_resolution_errors`) elaborates only the last
  (`The last overload gave the following error.`, with related information `The last overload is declared here.`),
  as TypeScript 5 does from four on. The error itself is stock's: the same place, code and first line
  (`TS2769: No overload matches this call.`), exit code 1 and nothing written
  (`rbxts_cli/tests/fixtures/overload_error`: `stock-stdout.txt` is stock's). Reproducing TypeScript 5's text would
  change tsrs's checker, which the fork does not do.

## Luau bugs
Luau engine bugs that stock roblox-ts's output triggers. rbxts-rs emits that output unchanged, as stock does (a 1:1
transpile; the bugs are Roblox's to fix): the `luau_o2` fixture pins it as stock's. Lune 0.10.5's Luau and Studio's
differ in places, as noted.

- **A builtin call's local argument at `--!optimize 2`.** Luau compiles a call of a builtin (`math.max`,
  `table.insert`, `bit32.bor`, `string.sub`, `tonumber`, `buffer.writestring`, …) with at most three arguments to
  FASTCALL, which at O2 reads a local argument's register after the later arguments ran. When a later argument
  reassigns that local through a closure, the builtin sees the new value; O0 and O1 (and JavaScript) see the old one.
  Studio (O2 by default for scripts; seen there with `table.insert`) and Lune agree:

  ```lua
  local a = 1
  local function rebind() a = 100; return 2 end
  print(math.max(a, rebind())) -- O0/O1: 2; O2: 100
  ```

  With four or more arguments the call is no FASTCALL, and `select` is one only as `select(n, ...)` (Lune 0.10.5):
  neither shows the bug.
- **A numeric `for` from a `-0` start.** In Studio, at O2 and so by default, `i` starts at `0`, not `-0`, both for a
  constant start and for one that is `-0` only at runtime; O0 and O1 give `-0`. Lune 0.10.5 shows it only for a
  constant start (the engine versions differ):

  ```lua
  for i = -0, 0 do print(1 / i) end -- O0/O1: -inf; O2: inf (Studio and Lune)
  local box = { start = -0 }
  for i = box.start, 0 do print(1 / i) end -- O0/O1: -inf; O2: inf in Studio, -inf in Lune 0.10.5
  ```

  Stock's output has such loops from `--optimizedLoops` (`for (let i = -0; …)`, a `const` of `-0`, `0 * -1`) and
  from `$range`.
