use bitflags::bitflags;

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
    pub struct Lowerings: u32 {
        const SIMD = 1 << 0;
        const REFERENCES = 1 << 1;
        const ALL = Self::SIMD.bits() | Self::REFERENCES.bits();
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OutputFormat {
    #[default]
    EsModule,
    Umd,
    Bare,
}

#[derive(Debug, Clone)]
pub struct ResourceLimits {
    pub max_input_bytes: usize,
    pub max_functions: usize,
    pub max_module_elements: usize,
    /// Maximum expanded local values in one function; zero disables the limit.
    pub max_function_locals: usize,
    /// Maximum decoded IR instructions in one function; zero disables the limit.
    pub max_function_ir: usize,
    pub max_total_ir: usize,
    pub max_memory_pages: u64,
    pub max_table_elements: u64,
    pub max_element_items: usize,
    pub max_segment_bytes: usize,
    pub max_js_bytes: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_input_bytes: 256 * 1024 * 1024,
            max_functions: 100_000,
            max_module_elements: 250_000,
            max_function_locals: 100_000,
            max_function_ir: 250_000,
            max_total_ir: 2_000_000,
            max_memory_pages: 32_768,
            max_table_elements: 1_000_000,
            max_element_items: 1_000_000,
            max_segment_bytes: 256 * 1024 * 1024,
            max_js_bytes: 512 * 1024 * 1024,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CompileOptions {
    pub lowerings: Lowerings,
    pub preserve_traps: bool,
    pub output_format: OutputFormat,
    pub global_name: String,
    pub limits: ResourceLimits,
}

impl Default for CompileOptions {
    fn default() -> Self {
        Self {
            lowerings: Lowerings::empty(),
            preserve_traps: true,
            output_format: OutputFormat::EsModule,
            global_name: "Wasm2AsmModule".into(),
            limits: ResourceLimits::default(),
        }
    }
}

impl Lowerings {
    pub fn parse(values: &[String], all: bool) -> Result<Self, String> {
        if all {
            return Ok(Self::ALL);
        }
        let mut result = Self::empty();
        for value in values {
            result |= match value.as_str() {
                "simd" => Self::SIMD,
                "references" => Self::REFERENCES,
                "all" => Self::ALL,
                _ => {
                    return Err(format!(
                        "unknown lowering {value:?}; expected simd, references, or all"
                    ));
                }
            };
        }
        Ok(result)
    }
}
