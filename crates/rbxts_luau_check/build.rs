//! Builds Luau's sources (the pinned luau0-src release) into static libraries, and the shim that keeps C++
//! exceptions on the C++ side (`src/shim.cpp`), and links them (the shim first: it calls into Luau).

fn main() {
    println!("cargo:rerun-if-changed=src/shim.cpp");
    cc::Build::new().cpp(true).std("c++17").warnings(true).file("src/shim.cpp").compile("rbxts_luau_shim");
    let artifacts = luau0_src::Build::new().build();
    artifacts.print_cargo_metadata();
}
