//! Embeds `carronade.manifest`: a console exe, so a shell waits for it and closes its piped stdin, that gets no console
//! window of its own when a hotkey starts it (Windows 11 24H2 and later,
//! <https://learn.microsoft.com/en-us/windows/console/console-allocation-policy>). Also embeds a version resource with
//! the product name and version from `Cargo.toml`, which code signing requires.
//!
//! Links the VC++ runtime statically and the Universal CRT, which ships with Windows, dynamically (the hybrid CRT,
//! <https://github.com/microsoft/WindowsAppSDK/blob/main/docs/Coding-Guidelines/HybridCRT.md>), so the exe runs
//! without the VC++ redistributable. A `crt-static` target feature in `.cargo/config.toml` would not do: config is
//! found from the current directory, not `--manifest-path`, and `cargo install` from crates.io ignores it.

fn main() -> std::io::Result<()> {
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "\\carronade.manifest");
    println!("cargo::rerun-if-changed=carronade.manifest");
    println!("cargo::rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo::rustc-link-arg-bins=/MANIFESTINPUT:{manifest}");
    for library in ["msvcrt.lib", "vcruntime.lib", "libucrt.lib"] {
        println!("cargo::rustc-link-arg=/NODEFAULTLIB:{library}");
    }
    for library in ["libcmt.lib", "libvcruntime.lib", "ucrt.lib"] {
        println!("cargo::rustc-link-arg=/DEFAULTLIB:{library}");
    }
    winresource::WindowsResource::new().compile()
}
