use clap::Parser;
use std::path::PathBuf;

#[derive(Debug, Parser)]
#[command(
    name = "wasm2asm",
    version,
    about = "Convert WebAssembly binaries to strictly validated asm.js"
)]
pub struct Cli {
    /// Input .wasm file, or - for stdin.
    pub input: PathBuf,

    /// Output JavaScript file, or - for stdout.
    #[arg(short = 'o', long = "output", default_value = "-")]
    pub output: PathBuf,

    /// Enable one high-cost lowering. Repeatable: simd, references, all.
    #[arg(long = "enable-lowering", value_name = "FEATURE")]
    pub enable_lowering: Vec<String>,

    /// Enable all implemented high-cost lowerings.
    #[arg(long)]
    pub enable_all_lowerings: bool,

    /// Permit JavaScript behavior where WebAssembly would trap.
    #[arg(long)]
    pub fast: bool,

    #[arg(long, default_value_t = 256 * 1024 * 1024)]
    pub max_input_bytes: usize,
    #[arg(long, default_value_t = 100_000)]
    pub max_functions: usize,
    #[arg(long, default_value_t = 250_000)]
    pub max_module_elements: usize,
    #[arg(long, default_value_t = 100_000)]
    pub max_function_ir: usize,
    #[arg(long, default_value_t = 2_000_000)]
    pub max_total_ir: usize,
    #[arg(long, default_value_t = 32_768)]
    pub max_memory_pages: u64,
    #[arg(long, default_value_t = 1_000_000)]
    pub max_table_elements: u64,
    #[arg(long, default_value_t = 1_000_000)]
    pub max_element_items: usize,
    #[arg(long, default_value_t = 256 * 1024 * 1024)]
    pub max_segment_bytes: usize,
    #[arg(long, default_value_t = 512 * 1024 * 1024)]
    pub max_js_bytes: usize,

    /// Print detected WebAssembly features and exit.
    #[arg(long)]
    pub print_module_features: bool,
}
