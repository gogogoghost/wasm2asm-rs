use std::fmt;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorKind {
    InvalidInput,
    UnsupportedFeature,
    ResourceLimit,
    Internal,
    Io,
}

#[derive(Debug, Error)]
#[error("{message}")]
pub struct CompileError {
    pub kind: ErrorKind,
    pub message: String,
    pub offset: Option<usize>,
}

impl CompileError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            offset: None,
        }
    }

    pub fn at(kind: ErrorKind, offset: usize, message: impl Into<String>) -> Self {
        Self {
            kind,
            message: message.into(),
            offset: Some(offset),
        }
    }

    pub fn unsupported(feature: &str, offset: Option<usize>, reason: impl fmt::Display) -> Self {
        let mut message =
            format!("wasm2asm: unsupported WebAssembly feature: {feature}\n  reason: {reason}");
        if let Some(offset) = offset {
            message.push_str(&format!("\n  byte offset: 0x{offset:x}"));
        }
        Self {
            kind: ErrorKind::UnsupportedFeature,
            message,
            offset,
        }
    }

    pub fn limit(resource: &str, value: impl fmt::Display, limit: impl fmt::Display) -> Self {
        Self::new(
            ErrorKind::ResourceLimit,
            format!(
                "wasm2asm: unsupported WebAssembly feature: resource limit\n  instruction: {resource}\n  reason: value is {value}; configured limit is {limit}"
            ),
        )
    }
}

impl From<wasmparser::BinaryReaderError> for CompileError {
    fn from(error: wasmparser::BinaryReaderError) -> Self {
        Self::at(
            ErrorKind::InvalidInput,
            usize::try_from(error.offset()).unwrap_or(usize::MAX),
            format!("wasm2asm: invalid WebAssembly input: {}", error.message()),
        )
    }
}

impl From<std::io::Error> for CompileError {
    fn from(error: std::io::Error) -> Self {
        Self::new(ErrorKind::Io, format!("wasm2asm: I/O error: {error}"))
    }
}
