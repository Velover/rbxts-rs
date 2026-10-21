//! Builds Luau's sources (the pinned luau0-src release) into static libraries and links them.

fn main() {
    let artifacts = luau0_src::Build::new().build();
    artifacts.print_cargo_metadata();
}
