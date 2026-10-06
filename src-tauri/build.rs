fn main() {
    #[cfg(target_os = "macos")]
    add_macos_link_args();
    tauri_build::build()
}

#[cfg(target_os = "macos")]
fn add_macos_link_args() {
    add_swift_runtime_rpaths();
    add_clang_runtime();
}

#[cfg(target_os = "macos")]
fn add_clang_runtime() {
    // whisper.cpp's ggml-metal-device.m uses @available(macOS 15, *) for
    // MTLResidencySet. Clang 19+/Xcode 16+ emits a call to
    // ___isPlatformVersionAtLeast for that check, which lives in
    // libclang_rt.osx.a (SDK 15+/26+/27+). rustc drives the final link with
    // -nodefaultlibs and does not auto-link compiler-rt, so the symbol is
    // missing unless we add it explicitly. See:
    //   clang --print-runtime-dir -> .../lib/darwin/libclang_rt.osx.a
    let candidate = std::process::Command::new("xcrun")
        .args(["clang", "--print-runtime-dir"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| {
            let dir = String::from_utf8_lossy(&o.stdout).trim().to_string();
            std::path::PathBuf::from(dir).join("libclang_rt.osx.a")
        })
        .filter(|p| p.is_file());

    if let Some(path) = candidate {
        println!("cargo:rustc-link-arg={}", path.display());
        return;
    }

    // Fallback: ask clang for its resource dir (older flag spelling).
    if let Ok(output) = std::process::Command::new("xcrun")
        .args(["clang", "-print-runtime-dir"])
        .output()
    {
        if output.status.success() {
            let dir = String::from_utf8_lossy(&output.stdout).trim().to_string();
            let p = std::path::PathBuf::from(dir).join("libclang_rt.osx.a");
            if p.is_file() {
                println!("cargo:rustc-link-arg={}", p.display());
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn add_swift_runtime_rpaths() {
    // ScreenCaptureKit's thin bridge is Swift. Dependency build-script linker
    // arguments do not reliably propagate to every Cargo target (notably the
    // Tauri bin test), so add the active toolchain paths at the final link too.
    println!("cargo:rustc-link-arg=-Wl,-rpath,/usr/lib/swift");
    let output = std::process::Command::new("xcrun")
        .args(["--find", "swiftc"])
        .output();
    let Ok(output) = output else { return };
    if !output.status.success() {
        return;
    }
    let swiftc = String::from_utf8_lossy(&output.stdout);
    let Some(toolchain) = std::path::Path::new(swiftc.trim()).ancestors().nth(3) else {
        return;
    };
    for suffix in ["usr/lib/swift/macosx", "usr/lib/swift-5.5/macosx"] {
        let path = toolchain.join(suffix);
        if path.is_dir() {
            println!("cargo:rustc-link-arg=-Wl,-rpath,{}", path.display());
        }
    }
}
