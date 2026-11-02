use std::{env, path::PathBuf};

fn main() {
    if env::var("CARGO_CFG_TARGET_FAMILY").as_deref() != Ok("unix") {
        return;
    }
    let root = PathBuf::from("vendor/airplay");
    println!("cargo:rerun-if-changed=vendor/airplay");
    let includes = [
        "src",
        "libraop/src",
        "libraop/crosstools/src",
        "libraop/dmap-parser",
        "libraop/libmdns/mdnssd",
        "libraop/libcodecs/addons",
        "libraop/libcodecs/alac/codec",
    ];
    let configure = || {
        let mut build = cc::Build::new();
        build.includes(includes.iter().map(|p| root.join(p)));
        build.include(env::var_os("DEP_OPENSSL_INCLUDE").expect("vendored OpenSSL headers"));
        build
            .define("NDEBUG", None)
            .define("_GNU_SOURCE", None)
            .define("OPENSSL_SUPPRESS_DEPRECATED", None)
            .define("SSL_STATIC_LIB", None)
            .define("CLIAIRPLAY_VERSION", Some("\"0.5.8\""));
        // Unmodified upstream sources are built separately from Rust lint checks.
        build.warnings(false);
        build
    };
    let mut c = configure();
    for source in [
        "libraop/src/raop_client.c",
        "libraop/src/rtsp_client.c",
        "libraop/src/aes.c",
        "libraop/src/aes_ctr.c",
        "libraop/src/alac.c",
        "libraop/dmap-parser/dmap_parser.c",
        "libraop/libmdns/mdnssd/mdnssd.c",
        "libraop/crosstools/src/cross_log.c",
        "libraop/crosstools/src/cross_ssl.c",
        "libraop/crosstools/src/cross_net.c",
        "libraop/crosstools/src/platform.c",
        "src/ap2_client.c",
        "src/ap2_hap.c",
        "src/ap2_io.c",
        "src/ap2_ptp.c",
        "src/ap2_ptp_shm.c",
        "src/ap2_plist.c",
        "src/ap2_mrp.c",
        "src/ap2_session.c",
        "src/host_clock.c",
        "src/raop_session.c",
        "src/artwork.c",
        "src/announce.c",
        "libraop/libcodecs/alac/codec/EndianPortable.c",
        "libraop/libcodecs/alac/codec/ALACBitUtilities.c",
        "libraop/libcodecs/alac/codec/ag_dec.c",
        "libraop/libcodecs/alac/codec/ag_enc.c",
        "libraop/libcodecs/alac/codec/dp_dec.c",
        "libraop/libcodecs/alac/codec/dp_enc.c",
        "libraop/libcodecs/alac/codec/matrix_dec.c",
        "libraop/libcodecs/alac/codec/matrix_enc.c",
    ] {
        c.file(root.join(source));
    }
    c.compile("textamp_airplay_c");
    configure()
        .define("main", Some("textamp_airplay_main"))
        .file(root.join("src/cliairplay.c"))
        .compile("textamp_airplay_entry");
    configure()
        .define("gettime_us", Some("crosstools_gettime_us"))
        .file(root.join("libraop/crosstools/src/cross_util.c"))
        .compile("textamp_airplay_clock");
    let mut cpp = configure();
    cpp.cpp(true).std("c++17");
    for source in [
        "libraop/src/pairing.cpp",
        "libraop/src/bplist.cpp",
        "src/ap2_bplist.cpp",
        "src/alac_ext.cpp",
        "libraop/libcodecs/alac/codec/ALACEncoder.cpp",
        "libraop/libcodecs/alac/codec/ALACDecoder.cpp",
    ] {
        cpp.file(root.join(source));
    }
    cpp.compile("textamp_airplay_cpp");
}
