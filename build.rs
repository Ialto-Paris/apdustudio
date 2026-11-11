// SPDX-License-Identifier: Apache-2.0
// Copyright 2026 Ialto

//! On Windows, puts the icon and the version details into the executable, so Explorer, the
//! taskbar pin and the file's properties show them. Other systems take the icon from the window.

fn main() {
    println!("cargo:rerun-if-changed=assets/icon.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("assets/icon.ico")
        .set("ProductName", "ApduStudio")
        .set(
            "FileDescription",
            "ApduStudio - PC/SC smart card test utility",
        )
        .set("OriginalFilename", "ApduStudio.exe")
        .set("CompanyName", "Ialto")
        .set(
            "LegalCopyright",
            "Copyright 2026 Ialto. Apache License 2.0.",
        );
    if let Err(e) = res.compile() {
        // A build without the icon would look like a different program: stop and say why.
        panic!(
            "can't put the icon into the executable ({e}). The Windows SDK's resource compiler \
             (rc.exe) is needed: install the Windows SDK, or build from a Developer Command Prompt."
        );
    }
}
