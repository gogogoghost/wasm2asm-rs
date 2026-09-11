use wasmparser::BlockType;

#[cfg(test)]
#[path = "ir/tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValType {
    I32,
    I64,
    F32,
    F64,
    V128,
    FuncRef(Option<u32>),
}

impl ValType {
    pub fn is_scalar(self) -> bool {
        matches!(self, Self::I32 | Self::I64 | Self::F32 | Self::F64)
    }
}

#[derive(Debug, Clone)]
pub struct FuncType {
    pub params: Vec<ValType>,
    pub results: Vec<ValType>,
}

#[derive(Debug, Clone)]
pub struct Import {
    pub module: String,
    pub name: String,
    pub kind: ImportKind,
}

#[derive(Debug, Clone)]
pub enum ImportKind {
    Func(u32),
    Memory(MemoryType),
    Table(TableType),
    Global(GlobalType),
    Tag(u32),
}

#[derive(Debug, Clone, Copy)]
pub struct MemoryType {
    pub memory64: bool,
    pub shared: bool,
    pub initial: u64,
    pub maximum: Option<u64>,
    pub page_size_log2: u32,
}

#[derive(Debug, Clone, Copy)]
pub struct TableType {
    pub table64: bool,
    pub initial: u64,
    pub maximum: Option<u64>,
    pub typed_function: Option<u32>,
}

#[derive(Debug, Clone, Copy)]
pub struct GlobalType {
    pub ty: ValType,
    pub mutable: bool,
}

#[derive(Debug, Clone)]
pub struct Global {
    pub ty: GlobalType,
    pub init: ConstExpr,
}

#[derive(Debug, Clone)]
pub enum ConstExpr {
    I32(i32),
    I64(i64),
    F32(u32),
    F64(u64),
    GlobalGet(u32),
    RefNull,
    RefFunc(u32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    Func,
    Table,
    Memory,
    Global,
    Tag,
}

#[derive(Debug, Clone)]
pub struct Export {
    pub name: String,
    pub kind: ExportKind,
    pub index: u32,
}

#[derive(Debug, Clone)]
pub enum ElementMode {
    Active { table: u32, offset: ConstExpr },
    Passive,
    Declared,
}

#[derive(Debug, Clone)]
pub struct Element {
    pub mode: ElementMode,
    pub items: Vec<Option<u32>>,
}

#[derive(Debug, Clone)]
pub enum DataMode {
    Active { memory: u32, offset: ConstExpr },
    Passive,
}

#[derive(Debug, Clone)]
pub struct DataSegment {
    pub mode: DataMode,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Function {
    pub type_index: u32,
    pub locals: Vec<ValType>,
    pub body: Vec<Instr>,
    pub name: Option<String>,
}

#[derive(Debug, Clone)]
pub struct Instr {
    pub offset: usize,
    pub op: Op,
}

#[derive(Debug, Clone)]
pub enum Op {
    Unreachable,
    Nop,
    Block(BlockSig),
    Loop(BlockSig),
    If(BlockSig),
    Else,
    End,
    Br(u32),
    BrIf(u32),
    BrTable {
        targets: Vec<u32>,
        default: u32,
    },
    Return,
    Call(u32),
    CallIndirect {
        type_index: u32,
        table_index: u32,
    },
    ReturnCall(u32),
    ReturnCallIndirect {
        type_index: u32,
        table_index: u32,
    },
    Drop,
    Select(Option<ValType>),
    LocalGet(u32),
    LocalSet(u32),
    LocalTee(u32),
    GlobalGet(u32),
    GlobalSet(u32),
    Load(LoadOp, MemArg),
    Store(StoreOp, MemArg),
    MemorySize(u32),
    MemoryGrow(u32),
    MemoryInit {
        data: u32,
        memory: u32,
    },
    DataDrop(u32),
    MemoryCopy {
        dst: u32,
        src: u32,
    },
    MemoryFill(u32),
    TableGet(u32),
    TableSet(u32),
    TableSize(u32),
    TableGrow(u32),
    TableFill(u32),
    TableCopy {
        dst: u32,
        src: u32,
    },
    TableInit {
        elem: u32,
        table: u32,
    },
    ElemDrop(u32),
    RefNull(Option<u32>),
    RefIsNull,
    RefFunc(u32),
    CallRef(u32),
    RefTest(Option<u32>),
    RefCast(Option<u32>),
    I32Const(i32),
    I64Const(i64),
    F32Const(u32),
    F64Const(u64),
    Unary(UnaryOp),
    Binary(BinaryOp),
    Simd(SimdOp),
    Throw(u32),
    TryTable {
        sig: BlockSig,
        catches: Vec<CatchClause>,
    },
    Unsupported {
        feature: &'static str,
        instruction: String,
    },
}

#[derive(Debug, Clone, Copy)]
pub enum CatchClause {
    Tag { tag: u32, label: u32 },
    All { label: u32 },
}

#[derive(Debug, Clone)]
pub struct BlockSig {
    pub params: Vec<ValType>,
    pub results: Vec<ValType>,
}

impl BlockSig {
    pub fn from_block_type(block: BlockType, types: &[FuncType]) -> Option<Self> {
        match block {
            BlockType::Empty => Some(Self {
                params: Vec::new(),
                results: Vec::new(),
            }),
            BlockType::Type(ty) => Some(Self {
                params: Vec::new(),
                results: vec![convert_val_type(ty)?],
            }),
            BlockType::FuncType(index) => types.get(index as usize).map(|ty| Self {
                params: ty.params.clone(),
                results: ty.results.clone(),
            }),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MemArg {
    pub offset: u64,
    pub memory: u32,
}

#[derive(Debug, Clone, Copy)]
pub enum LoadOp {
    I32,
    I64,
    F32,
    F64,
    I32_8S,
    I32_8U,
    I32_16S,
    I32_16U,
    I64_8S,
    I64_8U,
    I64_16S,
    I64_16U,
    I64_32S,
    I64_32U,
}

#[derive(Debug, Clone, Copy)]
pub enum StoreOp {
    I32,
    I64,
    F32,
    F64,
    I32_8,
    I32_16,
    I64_8,
    I64_16,
    I64_32,
}

#[derive(Debug, Clone, Copy)]
pub enum UnaryOp {
    I32Eqz,
    I32Clz,
    I32Ctz,
    I32Popcnt,
    I64Eqz,
    I64Clz,
    I64Ctz,
    I64Popcnt,
    F32Abs,
    F32Neg,
    F32Ceil,
    F32Floor,
    F32Trunc,
    F32Nearest,
    F32Sqrt,
    F64Abs,
    F64Neg,
    F64Ceil,
    F64Floor,
    F64Trunc,
    F64Nearest,
    F64Sqrt,
    I32WrapI64,
    I32TruncF32S,
    I32TruncF32U,
    I32TruncF64S,
    I32TruncF64U,
    I64ExtendI32S,
    I64ExtendI32U,
    I64TruncF32S,
    I64TruncF32U,
    I64TruncF64S,
    I64TruncF64U,
    F32ConvertI32S,
    F32ConvertI32U,
    F32ConvertI64S,
    F32ConvertI64U,
    F32DemoteF64,
    F64ConvertI32S,
    F64ConvertI32U,
    F64ConvertI64S,
    F64ConvertI64U,
    F64PromoteF32,
    I32ReinterpretF32,
    I64ReinterpretF64,
    F32ReinterpretI32,
    F64ReinterpretI64,
    I32Extend8S,
    I32Extend16S,
    I64Extend8S,
    I64Extend16S,
    I64Extend32S,
    I32TruncSatF32S,
    I32TruncSatF32U,
    I32TruncSatF64S,
    I32TruncSatF64U,
    I64TruncSatF32S,
    I64TruncSatF32U,
    I64TruncSatF64S,
    I64TruncSatF64U,
}

#[derive(Debug, Clone, Copy)]
pub enum BinaryOp {
    I32Eq,
    I32Ne,
    I32LtS,
    I32LtU,
    I32GtS,
    I32GtU,
    I32LeS,
    I32LeU,
    I32GeS,
    I32GeU,
    I64Eq,
    I64Ne,
    I64LtS,
    I64LtU,
    I64GtS,
    I64GtU,
    I64LeS,
    I64LeU,
    I64GeS,
    I64GeU,
    F32Eq,
    F32Ne,
    F32Lt,
    F32Gt,
    F32Le,
    F32Ge,
    F64Eq,
    F64Ne,
    F64Lt,
    F64Gt,
    F64Le,
    F64Ge,
    I32Add,
    I32Sub,
    I32Mul,
    I32DivS,
    I32DivU,
    I32RemS,
    I32RemU,
    I32And,
    I32Or,
    I32Xor,
    I32Shl,
    I32ShrS,
    I32ShrU,
    I32Rotl,
    I32Rotr,
    I64Add,
    I64Sub,
    I64Mul,
    I64DivS,
    I64DivU,
    I64RemS,
    I64RemU,
    I64And,
    I64Or,
    I64Xor,
    I64Shl,
    I64ShrS,
    I64ShrU,
    I64Rotl,
    I64Rotr,
    F32Add,
    F32Sub,
    F32Mul,
    F32Div,
    F32Min,
    F32Max,
    F32Copysign,
    F64Add,
    F64Sub,
    F64Mul,
    F64Div,
    F64Min,
    F64Max,
    F64Copysign,
}

#[derive(Debug, Clone)]
pub enum SimdOp {
    Const([u8; 16]),
    I32x4Splat,
    I32x4Add,
    I32x4Sub,
    I32x4Mul,
    I32x4Shl,
    I32x4Extract(u8),
    I32x4Replace(u8),
    F32x4Splat,
    F32x4Add,
    F32x4Sub,
    F32x4Mul,
    F32x4Div,
    F32x4Abs,
    F32x4Neg,
    F32x4Sqrt,
    F32x4Extract(u8),
    F32x4Replace(u8),
    I8x16Shuffle([u8; 16]),
    V128And,
    V128Or,
    V128Xor,
    V128Not,
    V128Bitselect,
    V128AnyTrue,
    V128Load(MemArg),
    V128Store(MemArg),
    Other(String),
}

#[derive(Debug, Default)]
pub struct Module {
    pub types: Vec<FuncType>,
    pub imports: Vec<Import>,
    pub function_type_indices: Vec<u32>,
    pub functions: Vec<Function>,
    pub memories: Vec<MemoryType>,
    pub tables: Vec<TableType>,
    pub globals: Vec<Global>,
    pub global_types: Vec<GlobalType>,
    pub tags: Vec<u32>,
    pub imported_function_count: u32,
    pub imported_memory_count: u32,
    pub imported_table_count: u32,
    pub imported_global_count: u32,
    pub exports: Vec<Export>,
    pub elements: Vec<Element>,
    pub data: Vec<DataSegment>,
    pub start: Option<u32>,
    pub features: FeatureUse,
    pub total_instructions: usize,
}

#[derive(Debug, Default, Clone)]
pub struct FeatureUse {
    pub i64: bool,
    pub bulk_memory: bool,
    pub multi_value: bool,
    pub reference_types: bool,
    pub typed_references: bool,
    pub simd: bool,
    pub tail_call: bool,
    pub exceptions: bool,
    pub memory64: bool,
    pub multi_memory: bool,
    pub custom_page_sizes: bool,
    pub threads: bool,
    pub gc: bool,
    pub unsupported: Vec<String>,
}

pub fn convert_val_type(ty: wasmparser::ValType) -> Option<ValType> {
    use wasmparser::{HeapType, ValType as W};
    match ty {
        W::I32 => Some(ValType::I32),
        W::I64 => Some(ValType::I64),
        W::F32 => Some(ValType::F32),
        W::F64 => Some(ValType::F64),
        W::V128 => Some(ValType::V128),
        W::Ref(r) => match r.heap_type() {
            HeapType::Abstract {
                ty: wasmparser::AbstractHeapType::Func | wasmparser::AbstractHeapType::NoFunc,
                ..
            } => Some(ValType::FuncRef(None)),
            HeapType::Concrete(index) | HeapType::Exact(index) => {
                Some(ValType::FuncRef(index.as_module_index()))
            }
            _ => None,
        },
    }
}
