use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

fn output_path(input: &Path, requested: Option<&Path>) -> PathBuf {
    requested
        .map(Path::to_path_buf)
        .unwrap_or_else(|| input.with_extension("wasm"))
}

fn run() -> Result<(), String> {
    let mut args = env::args_os().skip(1);
    let input = args.next().map(PathBuf::from).ok_or_else(|| {
        "usage: cargo run --example compile_wat -- INPUT.wat [OUTPUT.wasm|-]".to_string()
    })?;
    let output = args.next().map(PathBuf::from);
    if args.next().is_some() {
        return Err("expected one input and at most one output path".into());
    }

    let wasm = wat::parse_file(&input).map_err(|error| format!("{}: {error}", input.display()))?;
    let output = output_path(&input, output.as_deref());
    if output == Path::new("-") {
        io::stdout()
            .write_all(&wasm)
            .map_err(|error| error.to_string())?;
    } else {
        if let Some(parent) = output.parent() {
            fs::create_dir_all(parent).map_err(|error| format!("{}: {error}", parent.display()))?;
        }
        fs::write(&output, wasm).map_err(|error| format!("{}: {error}", output.display()))?;
        println!("{}", output.display());
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
