use std::env;
use std::error::Error;
use std::fs;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn Error>> {
    let destination = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src/lib/generated/ipc.ts");
    let mut args = env::args_os().skip(1);
    let check_path = match (args.next(), args.next(), args.next()) {
        (None, None, None) => None,
        (Some(arg), path, None) if arg == "--check" => {
            Some(path.map_or_else(|| destination.clone(), PathBuf::from))
        }
        _ => return Err("Usage: export-ipc [--check [bindings-file]]".into()),
    };
    let temporary = tempfile::tempdir()?;
    let generated = temporary.path().join("ipc.ts");
    tauri_appkokoro_engine_lib::ipc::export(&generated)?;
    let contents = fs::read(&generated)?;
    if let Some(path) = check_path {
        let committed = fs::read_to_string(&path).unwrap_or_default();
        if committed.replace("\r\n", "\n").as_bytes() != contents {
            return Err(format!(
                "IPC bindings are missing or stale at {}. Run npm run generate:ipc and commit src/lib/generated/ipc.ts.",
                path.display()
            ).into());
        }
    } else {
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent)?;
        }
        fs::write(&destination, contents)?;
    }
    Ok(())
}
