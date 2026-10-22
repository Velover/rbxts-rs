//! `rbxtsc-fw`: `rbxtsc` with Flamework compiled in (DESIGN §11.6). It takes roblox-ts's `build` arguments and
//! returns its exit codes; Flamework is on for a project whose tsconfig lists `@flamework-experimental/transformer`
//! in `compilerOptions.plugins`, and off otherwise.

#[global_allocator]
static GLOBAL: mimalloc_safe::MiMalloc = mimalloc_safe::MiMalloc;

fn main() -> std::process::ExitCode {
    rbxts_cli::Cli::new().plugin(flamework_rs::Flamework::new()).run()
}
