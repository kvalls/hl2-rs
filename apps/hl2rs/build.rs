//! macOS: embeds Info.plist into the hl2-bevy executable (`__TEXT,__info_plist`), so every build
//! of it, whatever its profile or features, carries the game's application metadata.
fn main() {
    println!("cargo:rerun-if-changed=Info.plist");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        let dir = std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets CARGO_MANIFEST_DIR");
        println!("cargo:rustc-link-arg-bins=-Wl,-sectcreate,__TEXT,__info_plist,{dir}/Info.plist");
    }
}
