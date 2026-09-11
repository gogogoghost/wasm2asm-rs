mod cli;

use clap::Parser;
use cli::Cli;
use std::fs;
use std::io::{self, Read, Write};
use std::path::Path;
use wasm2asm::{CompileOptions, Lowerings, ResourceLimits};

fn read_input(path: &Path, limit: usize) -> Result<Vec<u8>, String> {
    let mut data = Vec::new();
    if path == Path::new("-") {
        io::stdin()
            .take(limit.saturating_add(1) as u64)
            .read_to_end(&mut data)
            .map_err(|e| e.to_string())?;
    } else {
        data = fs::read(path).map_err(|e| format!("{}: {e}", path.display()))?;
    }
    if limit != 0 && data.len() > limit {
        return Err(format!("input exceeds configured limit {limit}"));
    }
    Ok(data)
}

fn run() -> Result<(), String> {
    let cli = Cli::parse();
    let lowerings = Lowerings::parse(&cli.enable_lowering, cli.enable_all_lowerings)?;
    let options = CompileOptions {
        lowerings,
        preserve_traps: !cli.fast,
        output_format: cli.format.into(),
        global_name: cli.global_name,
        limits: ResourceLimits {
            max_input_bytes: cli.max_input_bytes,
            max_functions: cli.max_functions,
            max_module_elements: cli.max_module_elements,
            max_function_locals: cli.max_function_locals,
            max_function_ir: cli.max_function_ir,
            max_total_ir: cli.max_total_ir,
            max_memory_pages: cli.max_memory_pages,
            max_table_elements: cli.max_table_elements,
            max_element_items: cli.max_element_items,
            max_segment_bytes: cli.max_segment_bytes,
            max_js_bytes: cli.max_js_bytes,
        },
    };
    let input = read_input(&cli.input, options.limits.max_input_bytes)?;
    if cli.print_module_features {
        println!(
            "{}",
            wasm2asm::inspect(&input, &options).map_err(|e| e.to_string())?
        );
        return Ok(());
    }
    let output = wasm2asm::compile(&input, &options).map_err(|e| e.to_string())?;
    if cli.output == Path::new("-") {
        io::stdout().write_all(&output).map_err(|e| e.to_string())?;
    } else {
        fs::write(&cli.output, output).map_err(|e| format!("{}: {e}", cli.output.display()))?;
    }
    Ok(())
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{error}");
        std::process::exit(1);
    }
}
