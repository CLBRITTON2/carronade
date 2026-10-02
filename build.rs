//! Embeds `carronade.manifest`: a console exe, so a shell waits for it and closes its piped stdin, that gets no console
//! window of its own when a hotkey starts it (Windows 11 24H2 and later,
//! https://learn.microsoft.com/en-us/windows/console/console-allocation-policy).

fn main() {
    let manifest = concat!(env!("CARGO_MANIFEST_DIR"), "\\carronade.manifest");
    println!("cargo::rerun-if-changed=carronade.manifest");
    println!("cargo::rustc-link-arg-bins=/MANIFEST:EMBED");
    println!("cargo::rustc-link-arg-bins=/MANIFESTINPUT:{manifest}");
}
