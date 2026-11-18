// Embeds the app icon and version info into pips.exe.
fn main() {
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("pips.ico");
    res.set("FileDescription", "Pips");
    res.set("ProductName", "Pips");
    res.compile().expect("embedding the app icon failed");
}
