//! Embeds the app icon (resource id 1, also loaded for the tray) and version info.

fn main() {
    println!("cargo:rerun-if-changed=assets/ebb.ico");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon_with_id("assets/ebb.ico", "1")
        .set("FileDescription", "Ebb")
        .set("ProductName", "Ebb")
        .set("OriginalFilename", "ebb.exe")
        .set("LegalCopyright", "© 2026 Gerrux");
    res.compile().expect("compile Windows resources");
}
