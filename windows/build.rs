// Embeds the app icon, version info and manifest into pips.exe.
fn main() {
    println!("cargo:rerun-if-env-changed=PIPS_VERSION");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    // CI sets PIPS_VERSION from the release tag, so Explorer's file properties match the release
    let version = std::env::var("PIPS_VERSION").unwrap_or_else(|_| std::env::var("CARGO_PKG_VERSION").unwrap());
    let mut res = winresource::WindowsResource::new();
    res.set_icon("pips.ico");
    res.set_manifest_file("pips.manifest");
    res.set("FileDescription", "Pips");
    res.set("ProductName", "Pips");
    res.set("ProductVersion", &version);
    res.set("FileVersion", &version);
    res.compile().expect("embedding the app icon failed");
}
