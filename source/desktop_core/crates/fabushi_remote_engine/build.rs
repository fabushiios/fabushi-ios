fn main() {
    println!("cargo:rerun-if-changed=native/macos_screen_capture.m");

    // CRITICAL iOS cross-compile guard:
    // On macos-15 cross-compiling to aarch64-apple-ios / aarch64-apple-ios-sim,
    // `#[cfg(target_os = "macos")]` evaluates TRUE (host OS is macOS), whereas
    // `CARGO_CFG_TARGET_OS` reflects the actual compilation target ("ios").
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        cc::Build::new()
            .file("native/macos_screen_capture.m")
            .flag("-fobjc-arc")
            .flag("-Wno-deprecated-declarations")
            .compile("fabushi_macos_screen_capture");

        println!("cargo:rustc-link-lib=framework=Foundation");
        println!("cargo:rustc-link-lib=framework=CoreGraphics");
        println!("cargo:rustc-link-lib=framework=CoreFoundation");
        println!("cargo:rustc-link-lib=framework=ImageIO");
        println!("cargo:rustc-link-lib=framework=ApplicationServices");
        println!("cargo:rustc-link-lib=framework=Security");
        println!("cargo:rustc-link-lib=framework=ScreenCaptureKit");
    }
}
