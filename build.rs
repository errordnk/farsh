//! On Windows (MSVC): FFmpeg's DLLs are delay-loaded, so farsh starts on a
//! machine without them and loads them only when a video's streams are asked
//! for (`src/srp/streams.rs`), and the three it calls (with swresample, which
//! avcodec imports) are put into the binary, zstd-compressed, from the vcpkg
//! tree it is linked against (so the DLLs always match the import libraries).

fn main() {
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if !(windows && msvc) {
        return;
    }
    // vcpkg's ffmpeg links every library of the port, so every one is delay-loaded
    for dll in ["avcodec-63.dll", "avformat-63.dll", "avutil-61.dll", "swresample-7.dll", "swscale-10.dll"] {
        println!("cargo:rustc-link-arg-bins=/DELAYLOAD:{dll}");
    }
    println!("cargo:rustc-link-arg-bins=delayimp.lib");

    println!("cargo:rerun-if-env-changed=VCPKG_ROOT");
    let root = std::env::var("VCPKG_ROOT").expect("VCPKG_ROOT (see .cargo/config.toml)");
    let bin = std::path::Path::new(&root).join("installed/x64-windows/bin");
    let out = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    // avcodec imports swresample itself (not delay-loaded): without it next to
    // the three, avcodec does not load and the first FFmpeg call ends farsh
    for dll in ["avutil-61.dll", "swresample-7.dll", "avcodec-63.dll", "avformat-63.dll"] {
        let source = bin.join(dll);
        println!("cargo:rerun-if-changed={}", source.display());
        let bytes = std::fs::read(&source).unwrap_or_else(|error| panic!("{}: {error}", source.display()));
        let compressed = zstd::encode_all(bytes.as_slice(), 19).unwrap();
        std::fs::write(out.join(format!("{dll}.zst")), compressed).unwrap();
    }
}
