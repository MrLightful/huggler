#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use sha2::{Digest, Sha256};
    use std::{env, fs, path::PathBuf, process::Command};

    println!("cargo:rerun-if-changed=build.rs");
    if env::var("CARGO_CFG_TARGET_OS")? != "macos" {
        return Ok(());
    }

    const URL: &str =
        "https://release.files.ghostty.org/libghostty-vt/1.0.1/ghostty-vt-1.0.1.xcframework.zip";
    const SHA256: &str = "c0f9631277f726154c8f521954d17cdc41aac4626fc0df8cf925a19ee58e829b";

    let out = PathBuf::from(env::var_os("OUT_DIR").expect("Cargo sets OUT_DIR"));
    let framework = "ghostty-vt.xcframework/macos-arm64_x86_64";
    let library = out.join(framework);
    let headers = library.join("Headers");
    let archive = out.join("ghostty-vt.zip");
    let stamp = out.join("ghostty-vt.sha256");
    if fs::read_to_string(&stamp).as_deref().ok() != Some(SHA256)
        || !library.join("libghostty-vt.a").is_file()
        || !headers.join("ghostty/vt.h").is_file()
    {
        let status = Command::new("curl")
            .args([
                "--fail",
                "--silent",
                "--show-error",
                "--location",
                "--retry",
                "2",
                "--max-time",
                "90",
                "--output",
            ])
            .arg(&archive)
            .arg(URL)
            .status()?;
        assert!(status.success(), "download libghostty-vt 1.0.1");
        assert_eq!(
            format!("{:x}", Sha256::digest(fs::read(&archive)?)),
            SHA256,
            "libghostty-vt archive checksum"
        );
        let status = Command::new("unzip")
            .args(["-oq"])
            .arg(&archive)
            .arg(format!("{framework}/*"))
            .arg("-d")
            .arg(&out)
            .status()?;
        assert!(status.success(), "extract macOS libghostty-vt");
        fs::write(stamp, SHA256)?;
        fs::remove_file(archive)?;
    }

    bindgen::Builder::default()
        .header(headers.join("ghostty/vt.h").to_string_lossy())
        .clang_arg(format!("-I{}", headers.display()))
        .allowlist_function("ghostty_(terminal|render_state|cell|key|mouse|formatter|grid_ref|tracked_grid_ref|paste|focus|free).*")
        .allowlist_var("GHOSTTY_(MODS|STYLE_FLAG|STYLE_UNDERLINE)_.*")
        .allowlist_type("Ghostty(TerminalModeConfig|CursorStyle|TerminalScreen|MouseEncoderSize|RenderStateColors|RenderStateCursor|CellWide|Buffer)")
        .prepend_enum_name(false)
        .generate_comments(false)
        .derive_default(true)
        .generate()?
        .write_to_file(out.join("ghostty.rs"))?;
    println!("cargo:rustc-link-search=native={}", library.display());
    println!("cargo:rustc-link-lib=static=ghostty-vt");
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn main() {}
