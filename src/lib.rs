mod diagnostics;
mod emitter;
#[allow(dead_code)]
mod ir;
mod options;
mod parser;

pub use diagnostics::{CompileError, ErrorKind};
pub use options::{CompileOptions, Lowerings, OutputFormat, ResourceLimits};

pub fn inspect(input: &[u8], options: &CompileOptions) -> Result<String, CompileError> {
    let mut inspection_options = options.clone();
    inspection_options.lowerings = Lowerings::ALL;
    let module = parser::parse_module(input, &inspection_options)?;
    Ok(format!("{:#?}", module.features))
}

pub fn compile(input: &[u8], options: &CompileOptions) -> Result<Vec<u8>, CompileError> {
    let module = parser::parse_module(input, options)?;
    emitter::emit(&module, options)
}
