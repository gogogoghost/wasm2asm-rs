use clap::{Parser, ValueEnum};
use std::path::PathBuf;

#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum OutputFormatArg {
    Esm,
    Umd,
    Bare,
}

impl From<OutputFormatArg> for wasm2asm::OutputFormat {
    fn from(value: OutputFormatArg) -> Self {
        match value {
            OutputFormatArg::Esm => Self::EsModule,
            OutputFormatArg::Umd => Self::Umd,
            OutputFormatArg::Bare => Self::Bare,
        }
    }
}

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

    /// Integration format: esm for bundlers, umd for script/CommonJS, bare for embedding.
    #[arg(long = "format", value_enum, default_value = "esm")]
    pub format: OutputFormatArg,

    /// Browser global used by --format=umd.
    #[arg(long, default_value = "Wasm2AsmModule")]
    pub global_name: String,

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
    /// Maximum expanded local values in one function; 0 disables the limit.
    #[arg(long, default_value_t = 100_000)]
    pub max_function_locals: usize,
    /// Maximum decoded IR instructions in one function; 0 disables the limit.
    #[arg(long, default_value_t = 250_000)]
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
