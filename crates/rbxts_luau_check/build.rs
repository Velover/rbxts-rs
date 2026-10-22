//! Builds Luau's sources (the pinned luau0-src release) into static libraries, and the shim that keeps C++
//! exceptions on the C++ side (`src/shim.cpp`), and links them (the shim first: it calls into Luau).
//!
//! On MSVC, C++ exception handling must be asked for: neither cc nor luau0-src adds `/EH`, and without it a refused
//! compile (Luau throws its errors) unwinds without destroying the objects in between (they leak), and
//! `catch (...)` also catches structured exceptions such as access violations.
//! - Luau gets `/EHsc` (standard C++ exceptions; `extern "C"` functions don't throw), through luau0-src's
//!   `LUAU_CXXFLAGS`.
//! - The shim gets `/EHs` without `c`: it catches what `luau_compile`, an `extern "C"` function, throws, so the
//!   compiler must not assume `extern "C"` never throws. Under `/EHs`, `catch (...)` does not catch structured
//!   exceptions: a crash inside Luau stays a crash.
//!
//! Other toolchains enable C++ exceptions by default and get no extra flags.

fn main() {
    println!("cargo:rerun-if-changed=src/shim.cpp");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").is_ok_and(|env| env == "msvc");
    let mut shim = cc::Build::new();
    shim.cpp(true).std("c++17").warnings(true).file("src/shim.cpp");
    if msvc {
        shim.flag("/EHs");
        let flags = match std::env::var("LUAU_CXXFLAGS") {
            Ok(flags) if !flags.trim().is_empty() => format!("{flags} /EHsc"),
            _ => "/EHsc".to_owned(),
        };
        // SAFETY: a build script is single-threaded until here (luau0-src starts its compile threads below), so no
        // other thread reads or writes the environment while it changes.
        unsafe { std::env::set_var("LUAU_CXXFLAGS", flags) };
    }
    shim.compile("rbxts_luau_shim");
    let artifacts = luau0_src::Build::new().build();
    artifacts.print_cargo_metadata();
}
