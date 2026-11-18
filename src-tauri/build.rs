// pattern: Imperative Shell

use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Seek};
use std::path::{Path, PathBuf};
use tempfile::NamedTempFile;

/// ONNX Runtime version compatible with ort-sys 2.0.0-rc.11 (requires >= 1.23.x).
/// fastembed 5.13.0 pulls ort 2.0.0-rc.11 which rejects anything below 1.23.x.
const ORT_VERSION: &str = "1.23.0";
// Microsoft.ML.OnnxRuntime 1.23.0 on NuGet; update these when changing ORT_VERSION.
const ORT_NUGET_BYTES: u64 = 122_633_649;
const ORT_NUGET_SHA256: &str = "cadbf21358de4b6002eddce08329465071271d52bb1483779f6f09670af281fc";

fn main() {
    println!("cargo:rerun-if-env-changed=ORT_DYLIB_PATH");
    println!("cargo:rerun-if-env-changed=ORT_LIB_LOCATION");
    println!("cargo:rerun-if-env-changed=ORT_SKIP_DOWNLOAD");
    println!("cargo:rerun-if-changed=migrations");

    // --- Auto-download ONNX Runtime shared library for `ort` (load-dynamic) ---
    let project_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let target_dir = project_dir.join("target").join("ort-dist");

    let (lib_name, archive_url) = ort_platform_info();

    // Final destination: next to Cargo.toml so `tauri dev` can find it,
    // and also copied next to the compiled binary for production builds.
    let dest = project_dir.join(lib_name);
    if dest.exists() && !is_usable_dylib(&dest) {
        let _ = fs::remove_file(&dest);
    }

    if !is_usable_dylib(&dest) {
        if let Some(existing) = find_env_ort(lib_name) {
            println!(
                "cargo:warning=Using ONNX Runtime from {}",
                existing.display()
            );
            fs::copy(&existing, &dest)
                .expect("failed to copy configured ONNX Runtime library next to Cargo.toml");
        } else if env_flag("ORT_SKIP_DOWNLOAD") {
            panic!(
                "ORT_SKIP_DOWNLOAD is set, but no usable ONNX Runtime library was found. \
                 Set ORT_DYLIB_PATH or ORT_LIB_LOCATION."
            );
        } else {
            println!(
                "cargo:warning=Downloading ONNX Runtime v{} from GitHub...",
                ORT_VERSION
            );
            fs::create_dir_all(&target_dir).expect("failed to create ort-dist cache dir");

            let archive_path = target_dir.join(archive_filename());
            let download_result = if archive_path.exists() {
                Ok(())
            } else {
                download(&archive_url, &archive_path)
            };

            match download_result {
                Ok(()) => {
                    let extracted = extract_lib(&archive_path, &target_dir, lib_name);
                    fs::copy(&extracted, &dest)
                        .expect("failed to copy ONNX Runtime lib to project root");
                }
                Err(ureq::Error::StatusCode(404)) => {
                    println!("cargo:warning=ONNX Runtime GitHub asset returned 404; trying Microsoft NuGet");
                    install_ort_from_nuget(&target_dir, &dest, lib_name)
                        .expect("failed to install ONNX Runtime from Microsoft NuGet");
                }
                Err(error) => panic!("failed to download ONNX Runtime from {archive_url}: {error}"),
            }
            println!(
                "cargo:warning=ONNX Runtime v{} installed to {}",
                ORT_VERSION,
                dest.display()
            );
        }
    }

    println!("cargo:warning=ONNX Runtime available at {}", dest.display());

    // Windows NSIS bundles only configured resources; include the runtime DLL
    // before Tauri codegen resolves the platform-specific bundle resources.
    tauri_build::build();
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        // Tauri embeds the application manifest in the desktop binary. The
        // export example also links Common Controls and needs its own manifest.
        println!("cargo:rustc-link-arg-examples=/MANIFEST:EMBED");
        println!("cargo:rustc-link-arg-examples=/MANIFESTDEPENDENCY:type='win32' name='Microsoft.Windows.Common-Controls' version='6.0.0.0' processorArchitecture='*' publicKeyToken='6595b64144ccf1df' language='*'");
    }
    copy_to_binary_dir(&dest, lib_name);
}

fn is_usable_dylib(path: &Path) -> bool {
    path.is_file()
        && path
            .metadata()
            .map(|metadata| metadata.len() > 0)
            .unwrap_or(false)
}

fn find_env_ort(lib_name: &str) -> Option<PathBuf> {
    std::env::var_os("ORT_DYLIB_PATH")
        .map(PathBuf::from)
        .filter(|path| is_usable_dylib(path))
        .or_else(|| {
            std::env::var_os("ORT_LIB_LOCATION")
                .map(PathBuf::from)
                .map(|dir| dir.join(lib_name))
                .filter(|path| is_usable_dylib(path))
        })
}

fn env_flag(name: &str) -> bool {
    std::env::var(name)
        .map(|value| matches!(value.as_str(), "1" | "true" | "TRUE" | "yes" | "YES"))
        .unwrap_or(false)
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

fn download(url: &str, dest: &Path) -> Result<(), ureq::Error> {
    println!("cargo:warning=GET {}", url);
    let resp = ureq::get(url).call()?;
    let mut reader = resp.into_body().into_reader();
    let mut file = fs::File::create(dest).expect("failed to create archive file");
    io::copy(&mut reader, &mut file).expect("failed to write archive");
    Ok(())
}

fn install_ort_from_nuget(cache_dir: &Path, dest: &Path, lib_name: &str) -> io::Result<()> {
    let platform = match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("windows") => "win",
        Ok("linux") => "linux",
        Ok("macos") => "osx",
        _ => return Err(io::Error::other("unsupported ONNX Runtime target OS")),
    };
    let arch = match std::env::var("CARGO_CFG_TARGET_ARCH").as_deref() {
        Ok("aarch64") => "arm64",
        Ok("x86_64") => "x64",
        _ => {
            return Err(io::Error::other(
                "unsupported ONNX Runtime target architecture",
            ))
        }
    };
    let url = format!(
        "https://api.nuget.org/v3-flatcontainer/microsoft.ml.onnxruntime/{ORT_VERSION}/microsoft.ml.onnxruntime.{ORT_VERSION}.nupkg"
    );
    println!("cargo:warning=GET {url}");
    let response = ureq::get(&url).call().map_err(io::Error::other)?;
    let entry_name = format!("runtimes/{platform}-{arch}/native/{lib_name}");
    install_verified_nuget_library(
        response.into_body().into_reader(),
        cache_dir,
        dest,
        &entry_name,
    )
}

fn install_verified_nuget_library(
    reader: impl Read,
    cache_dir: &Path,
    dest: &Path,
    entry_name: &str,
) -> io::Result<()> {
    let mut reader = reader.take(ORT_NUGET_BYTES + 1);
    let mut package = NamedTempFile::new_in(cache_dir)?;
    let bytes = io::copy(&mut reader, &mut package)?;
    if bytes != ORT_NUGET_BYTES {
        return Err(io::Error::other(format!(
            "expected {ORT_NUGET_BYTES} bytes for the ONNX Runtime NuGet package, read {bytes}"
        )));
    }

    package.rewind()?;
    let mut hash = Sha256::new();
    io::copy(&mut package, &mut hash)?;
    if format!("{:x}", hash.finalize()) != ORT_NUGET_SHA256 {
        return Err(io::Error::other(
            "ONNX Runtime NuGet package SHA-256 mismatch",
        ));
    }

    let mut zip = zip::ZipArchive::new(package.reopen()?).map_err(io::Error::other)?;
    let mut entry = zip.by_name(entry_name).map_err(io::Error::other)?;
    let library_dir = dest.parent().expect("missing library directory");
    #[cfg(unix)]
    let mut library = {
        use std::os::unix::fs::PermissionsExt;

        tempfile::Builder::new()
            .permissions(fs::Permissions::from_mode(0o644))
            .tempfile_in(library_dir)?
    };
    #[cfg(not(unix))]
    let mut library = NamedTempFile::new_in(library_dir)?;
    io::copy(&mut entry, &mut library)?;
    library.persist(dest).map_err(|error| error.error)?;
    Ok(())
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
        if !entry.header().entry_type().is_file() {
            continue;
        }
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
            let _ = fs::copy(lib_path, &bin_dest);

            let deps_dest = target_profile_dir.join("deps").join(lib_name);
            if let Some(parent) = deps_dest.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::copy(lib_path, &deps_dest);

            let examples_dest = target_profile_dir.join("examples").join(lib_name);
            if let Some(parent) = examples_dest.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let _ = fs::copy(lib_path, &examples_dest);
        }
    }
}
