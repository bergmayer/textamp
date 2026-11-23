use std::env;

fn main() {
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos")
        && env::var_os("CARGO_FEATURE_DESKTOP").is_some()
    {
        println!("cargo:rerun-if-changed=src/desktop/native_icon.m");
        cc::Build::new()
            .file("src/desktop/native_icon.m")
            .flag("-fobjc-arc")
            .flag("-fblocks")
            .compile("textamp_desktop_icon");
        println!("cargo:rustc-link-lib=framework=AppKit");
        println!("cargo:rerun-if-changed=src/text_mode/vt.c");
        println!("cargo:rerun-if-changed=vendor/ghostty-vt/libghostty-vt.a");
        cc::Build::new()
            .file("src/text_mode/vt.c")
            .include("vendor/ghostty-vt/include")
            .flag("-std=c11")
            .compile("textamp_terminal_vt");
        println!("cargo:rustc-link-search=native=vendor/ghostty-vt");
        println!("cargo:rustc-link-lib=static=ghostty-vt");
        println!("cargo:rustc-link-lib=c++");
    }
}
