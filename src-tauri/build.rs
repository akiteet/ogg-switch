fn main() {
    tauri_build::build();

    // Windows: Embed Common Controls v6 manifest for test binaries
    //
    // When running `cargo test`, the generated test executables don't include
    // the standard Tauri application manifest. Without Common Controls v6,
    // `tauri::test` calls fail with STATUS_ENTRYPOINT_NOT_FOUND.
    //
    // This workaround:
    // 1. Embeds the manifest into test binaries via /MANIFEST:EMBED
    // 2. Uses /MANIFEST:NO for the main binary to avoid duplicate resources
    //    (Tauri already handles manifest embedding for the app binary)
    // Only MSVC understands /MANIFEST:* link args; MinGW (windows-gnu) ld rejects
    // them as unknown input files, so guard by target env to keep GNU builds linkable.
    #[cfg(all(target_os = "windows", target_env = "msvc"))]
    {
        let manifest_path = std::path::PathBuf::from(
            std::env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"),
        )
        .join("common-controls.manifest");
        let manifest_arg = format!("/MANIFESTINPUT:{}", manifest_path.display());

        println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg={}", manifest_arg);
        // Avoid duplicate manifest resources in binary builds.
        println!("cargo:rustc-link-arg-bins=/MANIFEST:NO");
        println!("cargo:rerun-if-changed={}", manifest_path.display());
    }

    // GNU (MinGW) 工具链：ld 不认 /MANIFEST:*，改用 windres 把 manifest 编成
    // COFF 资源对象，仅注入 test target（主 bin 的 manifest 由 tauri-build 处理）。
    // 缺 windres 只降级为警告：主程序不依赖本步骤，受影响的只有 cargo test。
    #[cfg(all(target_os = "windows", target_env = "gnu"))]
    {
        if let Err(err) = embed_common_controls_manifest_for_tests() {
            println!("cargo:warning=嵌入测试 manifest 失败（cargo test 可能无法启动）: {err}");
        }
    }
}

/// 让 `cargo test` 生成的测试二进制带上 Common Controls v6 manifest。
///
/// 测试二进制导入了 `comctl32.dll!TaskDialogIndirect`（v6 独有），而 Windows 默认
/// 按 exe 的 manifest 决定加载 System32 的 v5 还是 WinSxS 的 v6。没有 manifest 时
/// 加载 v5 → 符号缺失 → 启动即 STATUS_ENTRYPOINT_NOT_FOUND (0xC0000139)。
#[cfg(all(target_os = "windows", target_env = "gnu"))]
fn embed_common_controls_manifest_for_tests() -> Result<(), std::io::Error> {
    use std::path::PathBuf;
    use std::process::Command;

    let manifest_dir =
        PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("missing CARGO_MANIFEST_DIR"));
    let manifest_path = manifest_dir.join("common-controls.manifest");
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("missing OUT_DIR"));

    // .rc 里的文件名相对于 windres 的工作目录（下方设为 manifest_dir）
    let rc_path = out_dir.join("common-controls.rc");
    std::fs::write(&rc_path, "1 24 \"common-controls.manifest\"\n")?;

    let obj_path = out_dir.join("common-controls.res.o");
    let output = Command::new("windres")
        .current_dir(&manifest_dir)
        .arg(&rc_path)
        .arg("-O")
        .arg("coff")
        .arg("-o")
        .arg(&obj_path)
        .output()?;
    if !output.status.success() {
        return Err(std::io::Error::other(format!(
            "windres 执行失败: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        )));
    }

    // lib 的单元测试（cargo test --lib）kind 是 lib 而非 test，
    // `rustc-link-arg-tests` 命中不到它，必须用覆盖全部目标的 `rustc-link-arg`。
    // 清单内容与 tauri-build 给 bin 的一致，bin 链接时重复出现同一资源无害。
    println!("cargo:rustc-link-arg={}", obj_path.display());
    println!("cargo:rerun-if-changed={}", manifest_path.display());
    Ok(())
}
