//! Embeds the app icon and file details into nock.exe so Explorer, the taskbar
//! and the "Details" tab show Nock instead of the generic Windows icon.

fn main() {
    println!("cargo:rerun-if-changed=assets/notch.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/notch.ico");
    res.set("ProductName", "Nock");
    res.set("FileDescription", "Nock - a notch and top bar for Windows");
    res.set("OriginalFilename", "nock.exe");
    res.set("LegalCopyright", "Copyright (c) 2026 Akhilesh Varute. MIT License.");
    if let Err(e) = res.compile() {
        // keep building without an icon rather than failing (for example without the Windows SDK)
        println!("cargo:warning=could not embed the exe icon: {e}");
    }
}
