use std::fs;
use std::io;
use std::path::{Path, PathBuf};

/// ONNX Runtime version that matches ort-sys 2.0.0-rc.11 (used by ort 2.0.0-rc.9).
/// v1.21.1 only ships GPU builds; v1.21.0 is the latest with CPU-only archives.
const ORT_VERSION: &str = "1.21.0";

fn main() {
    // --- Tauri codegen (must always run) ---
    tauri_build::build();

    // --- Auto-download ONNX Runtime shared library for `ort` (load-dynamic) ---
    let project_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let target_dir = project_dir.join("target").join("ort-dist");

    let (lib_name, archive_url) = ort_platform_info();

    // Final destination: next to Cargo.toml so `tauri dev` can find it,
    // and also copied next to the compiled binary for production builds.
    let dest = project_dir.join(lib_name);
    if dest.exists() {
        println!(
            "cargo:warning=ONNX Runtime already present at {}",
            dest.display()
        );
        copy_to_binary_dir(&dest, lib_name);
        return;
    }

    println!(
        "cargo:warning=Downloading ONNX Runtime v{} from GitHub...",
        ORT_VERSION
    );
    fs::create_dir_all(&target_dir).expect("failed to create ort-dist cache dir");

    let archive_path = target_dir.join(archive_filename());
    if !archive_path.exists() {
        download(&archive_url, &archive_path);
    }

    let extracted = extract_lib(&archive_path, &target_dir, lib_name);
    fs::copy(&extracted, &dest).expect("failed to copy ONNX Runtime lib to project root");
    println!(
        "cargo:warning=ONNX Runtime v{} installed to {}",
        ORT_VERSION,
        dest.display()
    );

    copy_to_binary_dir(&dest, lib_name);
}

/// Returns (library_filename, download_url) for the current build target.
fn ort_platform_info() -> (&'static str, String) {
    let base = format!(
        "https://github.com/microsoft/onnxruntime/releases/download/v{}",
        ORT_VERSION
    );

    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();

    match (target_os.as_str(), target_arch.as_str()) {
        ("windows", "x86_64") => (
            "onnxruntime.dll",
            format!("{}/onnxruntime-win-x64-{}.zip", base, ORT_VERSION),
        ),
        ("windows", "aarch64") => (
            "onnxruntime.dll",
            format!("{}/onnxruntime-win-arm64-{}.zip", base, ORT_VERSION),
        ),
        ("macos", "aarch64") => (
            "libonnxruntime.dylib",
            format!("{}/onnxruntime-osx-arm64-{}.tgz", base, ORT_VERSION),
        ),
        ("macos", "x86_64") => (
            "libonnxruntime.dylib",
            format!("{}/onnxruntime-osx-x86_64-{}.tgz", base, ORT_VERSION),
        ),
        ("linux", "x86_64") => (
            "libonnxruntime.so",
            format!("{}/onnxruntime-linux-x64-{}.tgz", base, ORT_VERSION),
        ),
        ("linux", "aarch64") => (
            "libonnxruntime.so",
            format!("{}/onnxruntime-linux-aarch64-{}.tgz", base, ORT_VERSION),
        ),
        _ => panic!(
            "Unsupported platform: {}-{}. Please manually place the ONNX Runtime library.",
            target_os, target_arch
        ),
    }
}

fn archive_filename() -> String {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH").unwrap();
    let arch_label = match (target_os.as_str(), target_arch.as_str()) {
        ("windows", "x86_64") => "win-x64",
        ("windows", "aarch64") => "win-arm64",
        ("macos", "aarch64") => "osx-arm64",
        ("macos", "x86_64") => "osx-x86_64",
        ("linux", "x86_64") => "linux-x64",
        ("linux", "aarch64") => "linux-aarch64",
        _ => "unknown",
    };
    let ext = if target_os == "windows" { "zip" } else { "tgz" };
    format!("onnxruntime-{}-{}.{}", arch_label, ORT_VERSION, ext)
}

fn download(url: &str, dest: &Path) {
    println!("cargo:warning=GET {}", url);
    let resp = ureq::get(url)
        .call()
        .expect("failed to download ONNX Runtime");
    let mut reader = resp.into_body().into_reader();
    let mut file = fs::File::create(dest).expect("failed to create archive file");
    io::copy(&mut reader, &mut file).expect("failed to write archive");
}

fn extract_lib(archive: &Path, cache_dir: &Path, lib_name: &str) -> PathBuf {
    let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap();
    if target_os == "windows" {
        extract_from_zip(archive, cache_dir, lib_name)
    } else {
        extract_from_tgz(archive, cache_dir, lib_name)
    }
}

fn extract_from_zip(archive: &Path, cache_dir: &Path, lib_name: &str) -> PathBuf {
    let file = fs::File::open(archive).expect("failed to open zip");
    let mut zip = zip::ZipArchive::new(file).expect("failed to read zip archive");

    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let name = entry.name().to_string();
        // The lib is at e.g. onnxruntime-win-x64-1.21.1/lib/onnxruntime.dll
        if name.ends_with(&format!("/lib/{}", lib_name)) {
            let out_path = cache_dir.join(lib_name);
            let mut out = fs::File::create(&out_path).expect("failed to create extracted file");
            io::copy(&mut entry, &mut out).expect("failed to extract file");
            return out_path;
        }
    }
    panic!(
        "Could not find {} in zip archive {}",
        lib_name,
        archive.display()
    );
}

fn extract_from_tgz(archive: &Path, cache_dir: &Path, lib_name: &str) -> PathBuf {
    let file = fs::File::open(archive).expect("failed to open tgz");
    let gz = flate2::read::GzDecoder::new(file);
    let mut tar = tar::Archive::new(gz);

    let suffix = format!("/lib/{}", lib_name);
    for entry in tar.entries().expect("failed to read tar entries") {
        let mut entry = entry.expect("failed to read tar entry");
        let path = entry
            .path()
            .expect("failed to read entry path")
            .to_path_buf();
        if path.to_string_lossy().ends_with(&suffix)
            || path
                .to_string_lossy()
                .contains(&format!("/lib/{}", lib_name.split('.').next().unwrap()))
        {
            let out_path = cache_dir.join(lib_name);
            let mut out = fs::File::create(&out_path).expect("failed to create extracted file");
            io::copy(&mut entry, &mut out).expect("failed to extract file");
            return out_path;
        }
    }
    panic!(
        "Could not find {} in tgz archive {}",
        lib_name,
        archive.display()
    );
}

/// Copy the library next to the compiled binary so it can be found at runtime.
fn copy_to_binary_dir(lib_path: &Path, lib_name: &str) {
    if let Ok(out_dir) = std::env::var("OUT_DIR") {
        // OUT_DIR is like target/debug/build/<pkg>/out — walk up to target/<profile>/
        let out = PathBuf::from(&out_dir);
        if let Some(target_profile_dir) = out.ancestors().nth(3) {
            let bin_dest = target_profile_dir.join(lib_name);
            if !bin_dest.exists() {
                let _ = fs::copy(lib_path, &bin_dest);
            }
        }
    }
}
