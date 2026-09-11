use crate::diagnostics::{CompileError, ErrorKind};
use crate::ir::*;
use crate::options::{CompileOptions, Lowerings};
use wasmparser::{
    AbstractHeapType, DataKind, ElementItems, ElementKind, ExternalKind, HeapType, Operator,
    Parser, Payload, TableInit, TypeRef, Validator, WasmFeatures,
};

#[cfg(test)]
#[path = "parser/tests.rs"]
mod tests;

pub fn parse_module(input: &[u8], options: &CompileOptions) -> Result<Module, CompileError> {
    if options.limits.max_input_bytes != 0 && input.len() > options.limits.max_input_bytes {
        return Err(CompileError::limit(
            "input bytes",
            input.len(),
            options.limits.max_input_bytes,
        ));
    }
    Validator::new_with_features(WasmFeatures::all()).validate_all(input)?;

    let mut module = Module::default();
    let mut next_defined_type = 0usize;
    let mut segment_bytes = 0usize;
    let mut element_items = 0usize;

    for payload in Parser::new(0).parse_all(input) {
        match payload? {
            Payload::Version { encoding, .. } => {
                if encoding != wasmparser::Encoding::Module {
                    return Err(CompileError::unsupported(
                        "component model",
                        None,
                        "only core WebAssembly modules can be converted",
                    ));
                }
            }
            Payload::TypeSection(reader) => {
                for ty in reader.into_iter_err_on_gc_types() {
                    let ty = ty.map_err(|error| {
                        CompileError::unsupported(
                            "garbage collection",
                            Some(usize::try_from(error.offset()).unwrap_or(usize::MAX)),
                            error.message(),
                        )
                    })?;
                    let params = ty
                        .params()
                        .iter()
                        .copied()
                        .map(convert_type)
                        .collect::<Result<Vec<_>, _>>()?;
                    let results = ty
                        .results()
                        .iter()
                        .copied()
                        .map(convert_type)
                        .collect::<Result<Vec<_>, _>>()?;
                    module.features.multi_value |=
                        results.len() > 1 || !params.is_empty() && results.len() > 1;
                    module.features.i64 |=
                        params.contains(&ValType::I64) || results.contains(&ValType::I64);
                    module.features.simd |=
                        params.contains(&ValType::V128) || results.contains(&ValType::V128);
                    module.features.typed_references |= params
                        .iter()
                        .chain(&results)
                        .any(|t| matches!(t, ValType::FuncRef(Some(_))));
                    module.types.push(FuncType { params, results });
                }
            }
            Payload::ImportSection(reader) => {
                for import in reader.into_imports() {
                    let import = import?;
                    let kind = match import.ty {
                        TypeRef::Func(index) | TypeRef::FuncExact(index) => {
                            module.imported_function_count += 1;
                            module.function_type_indices.push(index);
                            ImportKind::Func(index)
                        }
                        TypeRef::Memory(ty) => {
                            let ty = convert_memory(ty, &mut module.features);
                            module.imported_memory_count += 1;
                            module.memories.push(ty);
                            ImportKind::Memory(ty)
                        }
                        TypeRef::Table(ty) => {
                            let ty = convert_table(ty, &mut module.features)?;
                            module.imported_table_count += 1;
                            module.tables.push(ty);
                            ImportKind::Table(ty)
                        }
                        TypeRef::Global(ty) => {
                            if ty.shared {
                                module.features.threads = true;
                            }
                            let ty = GlobalType {
                                ty: convert_type(ty.content_type)?,
                                mutable: ty.mutable,
                            };
                            record_type_feature(ty.ty, &mut module.features);
                            module.imported_global_count += 1;
                            module.global_types.push(ty);
                            ImportKind::Global(ty)
                        }
                        TypeRef::Tag(ty) => {
                            module.features.exceptions = true;
                            module.tags.push(ty.func_type_idx);
                            ImportKind::Tag(ty.func_type_idx)
                        }
                    };
                    module.imports.push(Import {
                        module: import.module.into(),
                        name: import.name.into(),
                        kind,
                    });
                }
            }
            Payload::FunctionSection(reader) => {
                for index in reader {
                    module.function_type_indices.push(index?);
                }
            }
            Payload::TableSection(reader) => {
                for table in reader {
                    let table = table?;
                    if !matches!(table.init, TableInit::RefNull) {
                        module.features.reference_types = true;
                    }
                    let ty = convert_table(table.ty, &mut module.features)?;
                    module.tables.push(ty);
                }
            }
            Payload::MemorySection(reader) => {
                for memory in reader {
                    let ty = convert_memory(memory?, &mut module.features);
                    module.memories.push(ty);
                }
            }
            Payload::GlobalSection(reader) => {
                for global in reader {
                    let global = global?;
                    let ty = GlobalType {
                        ty: convert_type(global.ty.content_type)?,
                        mutable: global.ty.mutable,
                    };
                    record_type_feature(ty.ty, &mut module.features);
                    let init = parse_const(global.init_expr.get_operators_reader())?;
                    module.global_types.push(ty);
                    module.globals.push(Global { ty, init });
                }
            }
            Payload::TagSection(reader) => {
                module.features.exceptions = true;
                for tag in reader {
                    module.tags.push(tag?.func_type_idx);
                }
            }
            Payload::ExportSection(reader) => {
                for export in reader {
                    let export = export?;
                    let kind = match export.kind {
                        ExternalKind::Func | ExternalKind::FuncExact => ExportKind::Func,
                        ExternalKind::Table => ExportKind::Table,
                        ExternalKind::Memory => ExportKind::Memory,
                        ExternalKind::Global => ExportKind::Global,
                        ExternalKind::Tag => ExportKind::Tag,
                    };
                    module.exports.push(Export {
                        name: export.name.into(),
                        kind,
                        index: export.index,
                    });
                }
            }
            Payload::StartSection { func, .. } => module.start = Some(func),
            Payload::ElementSection(reader) => {
                for element in reader {
                    let element = element?;
                    let mode = match element.kind {
                        ElementKind::Active {
                            table_index,
                            offset_expr,
                        } => ElementMode::Active {
                            table: table_index.unwrap_or(0),
                            offset: parse_const(offset_expr.get_operators_reader())?,
                        },
                        ElementKind::Passive => {
                            module.features.bulk_memory = true;
                            ElementMode::Passive
                        }
                        ElementKind::Declared => ElementMode::Declared,
                    };
                    let items = match element.items {
                        ElementItems::Functions(reader) => reader
                            .into_iter()
                            .map(|item| item.map(Some))
                            .collect::<Result<Vec<_>, _>>()?,
                        ElementItems::Expressions(ty, reader) => {
                            if !ty.is_func_ref() && ty.type_index().is_none() {
                                return Err(CompileError::unsupported(
                                    "non-function element segments",
                                    None,
                                    "only function references and null are supported",
                                ));
                            }
                            module.features.reference_types = true;
                            let mut items = Vec::new();
                            for expression in reader {
                                items.push(
                                    match parse_const(expression?.get_operators_reader())? {
                                        ConstExpr::RefNull => None,
                                        ConstExpr::RefFunc(index) => Some(index),
                                        _ => {
                                            return Err(CompileError::unsupported(
                                                "element expressions",
                                                None,
                                                "element item is not ref.func or ref.null",
                                            ));
                                        }
                                    },
                                );
                            }
                            items
                        }
                    };
                    element_items = element_items.saturating_add(items.len());
                    module.elements.push(Element { mode, items });
                }
            }
            Payload::DataSection(reader) => {
                for data in reader {
                    let data = data?;
                    segment_bytes = segment_bytes.saturating_add(data.data.len());
                    let mode = match data.kind {
                        DataKind::Passive => {
                            module.features.bulk_memory = true;
                            DataMode::Passive
                        }
                        DataKind::Active {
                            memory_index,
                            offset_expr,
                        } => DataMode::Active {
                            memory: memory_index,
                            offset: parse_const(offset_expr.get_operators_reader())?,
                        },
                    };
                    module.data.push(DataSegment {
                        mode,
                        bytes: data.data.to_vec(),
                    });
                }
            }
            Payload::CodeSectionEntry(body) => {
                let all_index = module.imported_function_count as usize + next_defined_type;
                let type_index = *module.function_type_indices.get(all_index).ok_or_else(|| {
                    CompileError::new(
                        ErrorKind::Internal,
                        "wasm2asm: function/code section mismatch",
                    )
                })?;
                next_defined_type += 1;
                let mut locals = Vec::new();
                for local in body.get_locals_reader()? {
                    let (count, ty) = local?;
                    let ty = convert_type(ty)?;
                    record_type_feature(ty, &mut module.features);
                    let count = usize::try_from(count)
                        .map_err(|_| CompileError::limit("function locals", count, usize::MAX))?;
                    if options.limits.max_function_locals != 0
                        && locals.len().saturating_add(count) > options.limits.max_function_locals
                    {
                        return Err(CompileError::limit(
                            "function locals",
                            locals.len().saturating_add(count),
                            options.limits.max_function_locals,
                        ));
                    }
                    locals.resize(locals.len() + count, ty);
                }
                let mut instructions = Vec::new();
                for item in body.get_operators_reader()?.into_iter_with_offsets() {
                    let (operator, offset) = item?;
                    let op = convert_operator(operator, &module.types, &mut module.features)?;
                    instructions.push(Instr {
                        offset: offset as usize,
                        op,
                    });
                    if options.limits.max_function_ir != 0
                        && instructions.len() > options.limits.max_function_ir
                    {
                        return Err(CompileError::limit(
                            "function IR expressions",
                            instructions.len(),
                            options.limits.max_function_ir,
                        ));
                    }
                }
                module.total_instructions =
                    module.total_instructions.saturating_add(instructions.len());
                module.functions.push(Function {
                    type_index,
                    locals,
                    body: instructions,
                    name: None,
                });
            }
            Payload::DataCountSection { .. }
            | Payload::CodeSectionStart { .. }
            | Payload::CustomSection(_)
            | Payload::End(_) => {}
            other => {
                return Err(CompileError::unsupported(
                    "unsupported module section",
                    other.as_section().map(|(_, r)| r.start as usize),
                    format!("payload {other:?}"),
                ));
            }
        }
    }

    classify_recursive_tail_calls(&mut module);
    check_limits(&module, segment_bytes, element_items, options)?;
    check_capabilities(&module, options)?;
    Ok(module)
}

fn classify_recursive_tail_calls(module: &mut Module) {
    if !module.features.tail_call {
        return;
    }
    let count = module.function_type_indices.len();
    let mut graph = vec![Vec::new(); count];
    let mut tail_edges = Vec::new();
    let mut indirect_tail = false;
    for (defined, function) in module.functions.iter().enumerate() {
        let caller = module.imported_function_count as usize + defined;
        for instruction in &function.body {
            match instruction.op {
                Op::Call(target) => graph[caller].push(target as usize),
                Op::ReturnCall(target) => {
                    graph[caller].push(target as usize);
                    tail_edges.push((caller, target as usize));
                }
                Op::ReturnCallIndirect { .. } => indirect_tail = true,
                _ => {}
            }
        }
    }
    module.features.tail_call = indirect_tail
        || tail_edges.into_iter().any(|(caller, target)| {
            let mut stack = vec![target];
            let mut seen = vec![false; count];
            while let Some(node) = stack.pop() {
                if node == caller {
                    return true;
                }
                if node >= count || seen[node] {
                    continue;
                }
                seen[node] = true;
                stack.extend(graph[node].iter().copied());
            }
            false
        });
}

fn check_limits(
    module: &Module,
    segment_bytes: usize,
    element_items: usize,
    options: &CompileOptions,
) -> Result<(), CompileError> {
    let limits = &options.limits;
    let functions = module.function_type_indices.len();
    if limits.max_functions != 0 && functions > limits.max_functions {
        return Err(CompileError::limit(
            "functions",
            functions,
            limits.max_functions,
        ));
    }
    let elements = functions
        + module.globals.len()
        + module.memories.len()
        + module.tables.len()
        + module.tags.len()
        + module.exports.len()
        + module.elements.len()
        + module.data.len();
    if limits.max_module_elements != 0 && elements > limits.max_module_elements {
        return Err(CompileError::limit(
            "module elements",
            elements,
            limits.max_module_elements,
        ));
    }
    if limits.max_total_ir != 0 && module.total_instructions > limits.max_total_ir {
        return Err(CompileError::limit(
            "total IR expressions",
            module.total_instructions,
            limits.max_total_ir,
        ));
    }
    if limits.max_segment_bytes != 0 && segment_bytes > limits.max_segment_bytes {
        return Err(CompileError::limit(
            "data segment bytes",
            segment_bytes,
            limits.max_segment_bytes,
        ));
    }
    if limits.max_element_items != 0 && element_items > limits.max_element_items {
        return Err(CompileError::limit(
            "element segment items",
            element_items,
            limits.max_element_items,
        ));
    }
    for memory in &module.memories {
        let equivalent_pages = memory
            .initial
            .saturating_mul(1u64 << memory.page_size_log2.saturating_sub(16));
        if limits.max_memory_pages != 0 && equivalent_pages > limits.max_memory_pages {
            return Err(CompileError::limit(
                "initial memory pages",
                equivalent_pages,
                limits.max_memory_pages,
            ));
        }
    }
    for table in &module.tables {
        if limits.max_table_elements != 0 && table.initial > limits.max_table_elements {
            return Err(CompileError::limit(
                "initial table elements",
                table.initial,
                limits.max_table_elements,
            ));
        }
    }
    Ok(())
}

fn check_capabilities(module: &Module, options: &CompileOptions) -> Result<(), CompileError> {
    let f = &module.features;
    if f.threads {
        return Err(CompileError::unsupported(
            "threads or shared memory",
            None,
            "asm.js cannot preserve atomic shared-memory semantics",
        ));
    }
    if f.gc {
        return Err(CompileError::unsupported(
            "garbage collection",
            None,
            "managed WebAssembly references have no portable asm.js representation",
        ));
    }
    if module.tables.len() > 1 {
        return Err(CompileError::unsupported(
            "multiple tables",
            None,
            "the asm.js runtime supports one internal function table",
        ));
    }
    if module.imported_table_count != 0
        || module.exports.iter().any(|e| e.kind == ExportKind::Table)
    {
        return Err(CompileError::unsupported(
            "imported or exported tables",
            None,
            "host functions do not expose canonical WebAssembly signature metadata",
        ));
    }
    if f.simd && !options.lowerings.contains(Lowerings::SIMD) {
        return Err(CompileError::unsupported(
            "simd",
            None,
            "enable with --enable-lowering=simd",
        ));
    }
    if f.tail_call {
        return Err(CompileError::unsupported(
            "recursive or indirect tail calls",
            None,
            "the implemented trampoline uses dynamic JavaScript and is not eligible for strict asm.js output",
        ));
    }
    if f.exceptions {
        return Err(CompileError::unsupported(
            "exception handling",
            None,
            "JavaScript try/catch and thrown objects are not valid inside a strict asm.js module",
        ));
    }
    if f.typed_references && !options.lowerings.contains(Lowerings::REFERENCES) {
        return Err(CompileError::unsupported(
            "typed function references",
            None,
            "enable with --enable-lowering=references",
        ));
    }
    if let Some(name) = f.unsupported.first() {
        return Err(CompileError::unsupported(
            "unsupported proposal",
            None,
            name,
        ));
    }
    Ok(())
}

fn convert_memory(ty: wasmparser::MemoryType, features: &mut FeatureUse) -> MemoryType {
    features.memory64 |= ty.memory64;
    features.threads |= ty.shared;
    features.custom_page_sizes |= ty.page_size_log2.is_some();
    MemoryType {
        memory64: ty.memory64,
        shared: ty.shared,
        initial: ty.initial,
        maximum: ty.maximum,
        page_size_log2: ty.page_size_log2(),
    }
}

fn convert_table(
    ty: wasmparser::TableType,
    features: &mut FeatureUse,
) -> Result<TableType, CompileError> {
    features.memory64 |= ty.table64;
    features.threads |= ty.shared;
    let typed_function = match ty.element_type.heap_type() {
        HeapType::Abstract {
            ty: AbstractHeapType::Func | AbstractHeapType::NoFunc,
            ..
        } => None,
        HeapType::Concrete(index) | HeapType::Exact(index) => {
            features.typed_references = true;
            index.as_module_index()
        }
        _ => {
            return Err(CompileError::unsupported(
                "non-function table",
                None,
                "only nullable function-reference tables are supported",
            ));
        }
    };
    Ok(TableType {
        table64: ty.table64,
        initial: ty.initial,
        maximum: ty.maximum,
        typed_function,
    })
}

fn convert_type(ty: wasmparser::ValType) -> Result<ValType, CompileError> {
    convert_val_type(ty).ok_or_else(|| {
        CompileError::unsupported("non-function references", None, format!("value type {ty}"))
    })
}

fn record_type_feature(ty: ValType, features: &mut FeatureUse) {
    match ty {
        ValType::I64 => features.i64 = true,
        ValType::V128 => features.simd = true,
        ValType::FuncRef(Some(_)) => {
            features.reference_types = true;
            features.typed_references = true;
        }
        ValType::FuncRef(None) => features.reference_types = true,
        _ => {}
    }
}

fn parse_const(mut reader: wasmparser::OperatorsReader<'_>) -> Result<ConstExpr, CompileError> {
    let (operator, _) = reader.read_with_offset()?;
    let value = match operator {
        Operator::I32Const { value } => ConstExpr::I32(value),
        Operator::I64Const { value } => ConstExpr::I64(value),
        Operator::F32Const { value } => ConstExpr::F32(value.bits()),
        Operator::F64Const { value } => ConstExpr::F64(value.bits()),
        Operator::GlobalGet { global_index } => ConstExpr::GlobalGet(global_index),
        Operator::RefNull { .. } => ConstExpr::RefNull,
        Operator::RefFunc { function_index } => ConstExpr::RefFunc(function_index),
        other => {
            return Err(CompileError::unsupported(
                "constant expressions",
                None,
                format!("instruction {other:?}"),
            ));
        }
    };
    if !matches!(reader.read()?, Operator::End) {
        return Err(CompileError::new(
            ErrorKind::InvalidInput,
            "wasm2asm: malformed constant expression",
        ));
    }
    Ok(value)
}

fn sig(blockty: wasmparser::BlockType, types: &[FuncType]) -> Result<BlockSig, CompileError> {
    BlockSig::from_block_type(blockty, types).ok_or_else(|| {
        CompileError::unsupported(
            "block type",
            None,
            "block uses a non-portable reference type",
        )
    })
}

fn memarg(value: wasmparser::MemArg) -> MemArg {
    MemArg {
        offset: value.offset,
        memory: value.memory,
    }
}

fn heap_type_index(hty: HeapType) -> Option<u32> {
    match hty {
        HeapType::Concrete(index) | HeapType::Exact(index) => index.as_module_index(),
        _ => None,
    }
}

#[allow(clippy::too_many_lines)]
fn convert_operator(
    operator: Operator<'_>,
    types: &[FuncType],
    features: &mut FeatureUse,
) -> Result<Op, CompileError> {
    use BinaryOp as B;
    use LoadOp as L;
    use Op::*;
    use StoreOp as S;
    use UnaryOp as U;
    let op = match operator {
        Operator::Unreachable => Unreachable,
        Operator::Nop => Nop,
        Operator::Block { blockty } => Block(sig(blockty, types)?),
        Operator::Loop { blockty } => Loop(sig(blockty, types)?),
        Operator::If { blockty } => If(sig(blockty, types)?),
        Operator::Else => Else,
        Operator::End => End,
        Operator::Br { relative_depth } => Br(relative_depth),
        Operator::BrIf { relative_depth } => BrIf(relative_depth),
        Operator::BrTable { targets } => BrTable {
            targets: targets.targets().collect::<Result<Vec<_>, _>>()?,
            default: targets.default(),
        },
        Operator::Return => Return,
        Operator::Call { function_index } => Call(function_index),
        Operator::CallIndirect {
            type_index,
            table_index,
        } => CallIndirect {
            type_index,
            table_index,
        },
        Operator::ReturnCall { function_index } => {
            features.tail_call = true;
            ReturnCall(function_index)
        }
        Operator::ReturnCallIndirect {
            type_index,
            table_index,
        } => {
            features.tail_call = true;
            ReturnCallIndirect {
                type_index,
                table_index,
            }
        }
        Operator::Drop => Drop,
        Operator::Select => Select(None),
        Operator::TypedSelect { ty } => Select(Some(convert_type(ty)?)),
        Operator::LocalGet { local_index } => LocalGet(local_index),
        Operator::LocalSet { local_index } => LocalSet(local_index),
        Operator::LocalTee { local_index } => LocalTee(local_index),
        Operator::GlobalGet { global_index } => GlobalGet(global_index),
        Operator::GlobalSet { global_index } => GlobalSet(global_index),
        Operator::I32Load { memarg: m } => Load(L::I32, memarg(m)),
        Operator::I64Load { memarg: m } => Load(L::I64, memarg(m)),
        Operator::F32Load { memarg: m } => Load(L::F32, memarg(m)),
        Operator::F64Load { memarg: m } => Load(L::F64, memarg(m)),
        Operator::I32Load8S { memarg: m } => Load(L::I32_8S, memarg(m)),
        Operator::I32Load8U { memarg: m } => Load(L::I32_8U, memarg(m)),
        Operator::I32Load16S { memarg: m } => Load(L::I32_16S, memarg(m)),
        Operator::I32Load16U { memarg: m } => Load(L::I32_16U, memarg(m)),
        Operator::I64Load8S { memarg: m } => Load(L::I64_8S, memarg(m)),
        Operator::I64Load8U { memarg: m } => Load(L::I64_8U, memarg(m)),
        Operator::I64Load16S { memarg: m } => Load(L::I64_16S, memarg(m)),
        Operator::I64Load16U { memarg: m } => Load(L::I64_16U, memarg(m)),
        Operator::I64Load32S { memarg: m } => Load(L::I64_32S, memarg(m)),
        Operator::I64Load32U { memarg: m } => Load(L::I64_32U, memarg(m)),
        Operator::I32Store { memarg: m } => Store(S::I32, memarg(m)),
        Operator::I64Store { memarg: m } => Store(S::I64, memarg(m)),
        Operator::F32Store { memarg: m } => Store(S::F32, memarg(m)),
        Operator::F64Store { memarg: m } => Store(S::F64, memarg(m)),
        Operator::I32Store8 { memarg: m } => Store(S::I32_8, memarg(m)),
        Operator::I32Store16 { memarg: m } => Store(S::I32_16, memarg(m)),
        Operator::I64Store8 { memarg: m } => Store(S::I64_8, memarg(m)),
        Operator::I64Store16 { memarg: m } => Store(S::I64_16, memarg(m)),
        Operator::I64Store32 { memarg: m } => Store(S::I64_32, memarg(m)),
        Operator::MemorySize { mem } => MemorySize(mem),
        Operator::MemoryGrow { mem } => MemoryGrow(mem),
        Operator::MemoryInit { data_index, mem } => {
            features.bulk_memory = true;
            MemoryInit {
                data: data_index,
                memory: mem,
            }
        }
        Operator::DataDrop { data_index } => {
            features.bulk_memory = true;
            DataDrop(data_index)
        }
        Operator::MemoryCopy { dst_mem, src_mem } => {
            features.bulk_memory = true;
            features.multi_memory |= dst_mem != 0 || src_mem != 0;
            MemoryCopy {
                dst: dst_mem,
                src: src_mem,
            }
        }
        Operator::MemoryFill { mem } => {
            features.bulk_memory = true;
            MemoryFill(mem)
        }
        Operator::TableGet { table } => {
            features.reference_types = true;
            TableGet(table)
        }
        Operator::TableSet { table } => {
            features.reference_types = true;
            TableSet(table)
        }
        Operator::TableSize { table } => {
            features.reference_types = true;
            TableSize(table)
        }
        Operator::TableGrow { table } => {
            features.reference_types = true;
            TableGrow(table)
        }
        Operator::TableFill { table } => {
            features.reference_types = true;
            TableFill(table)
        }
        Operator::TableCopy {
            dst_table,
            src_table,
        } => {
            features.bulk_memory = true;
            TableCopy {
                dst: dst_table,
                src: src_table,
            }
        }
        Operator::TableInit { elem_index, table } => {
            features.bulk_memory = true;
            TableInit {
                elem: elem_index,
                table,
            }
        }
        Operator::ElemDrop { elem_index } => {
            features.bulk_memory = true;
            ElemDrop(elem_index)
        }
        Operator::RefNull { hty } => {
            features.reference_types = true;
            RefNull(heap_type_index(hty))
        }
        Operator::RefIsNull => {
            features.reference_types = true;
            RefIsNull
        }
        Operator::RefFunc { function_index } => {
            features.reference_types = true;
            RefFunc(function_index)
        }
        Operator::CallRef { type_index } => {
            features.typed_references = true;
            CallRef(type_index)
        }
        Operator::RefTestNonNull { hty } | Operator::RefTestNullable { hty } => {
            features.typed_references = true;
            RefTest(heap_type_index(hty))
        }
        Operator::RefCastNonNull { hty } | Operator::RefCastNullable { hty } => {
            features.typed_references = true;
            RefCast(heap_type_index(hty))
        }
        Operator::I32Const { value } => I32Const(value),
        Operator::I64Const { value } => {
            features.i64 = true;
            I64Const(value)
        }
        Operator::F32Const { value } => F32Const(value.bits()),
        Operator::F64Const { value } => F64Const(value.bits()),

        Operator::I32Eqz => Unary(U::I32Eqz),
        Operator::I32Clz => Unary(U::I32Clz),
        Operator::I32Ctz => Unary(U::I32Ctz),
        Operator::I32Popcnt => Unary(U::I32Popcnt),
        Operator::I64Eqz => Unary(U::I64Eqz),
        Operator::I64Clz => Unary(U::I64Clz),
        Operator::I64Ctz => Unary(U::I64Ctz),
        Operator::I64Popcnt => Unary(U::I64Popcnt),
        Operator::F32Abs => Unary(U::F32Abs),
        Operator::F32Neg => Unary(U::F32Neg),
        Operator::F32Ceil => Unary(U::F32Ceil),
        Operator::F32Floor => Unary(U::F32Floor),
        Operator::F32Trunc => Unary(U::F32Trunc),
        Operator::F32Nearest => Unary(U::F32Nearest),
        Operator::F32Sqrt => Unary(U::F32Sqrt),
        Operator::F64Abs => Unary(U::F64Abs),
        Operator::F64Neg => Unary(U::F64Neg),
        Operator::F64Ceil => Unary(U::F64Ceil),
        Operator::F64Floor => Unary(U::F64Floor),
        Operator::F64Trunc => Unary(U::F64Trunc),
        Operator::F64Nearest => Unary(U::F64Nearest),
        Operator::F64Sqrt => Unary(U::F64Sqrt),
        Operator::I32WrapI64 => Unary(U::I32WrapI64),
        Operator::I32TruncF32S => Unary(U::I32TruncF32S),
        Operator::I32TruncF32U => Unary(U::I32TruncF32U),
        Operator::I32TruncF64S => Unary(U::I32TruncF64S),
        Operator::I32TruncF64U => Unary(U::I32TruncF64U),
        Operator::I64ExtendI32S => Unary(U::I64ExtendI32S),
        Operator::I64ExtendI32U => Unary(U::I64ExtendI32U),
        Operator::I64TruncF32S => Unary(U::I64TruncF32S),
        Operator::I64TruncF32U => Unary(U::I64TruncF32U),
        Operator::I64TruncF64S => Unary(U::I64TruncF64S),
        Operator::I64TruncF64U => Unary(U::I64TruncF64U),
        Operator::F32ConvertI32S => Unary(U::F32ConvertI32S),
        Operator::F32ConvertI32U => Unary(U::F32ConvertI32U),
        Operator::F32ConvertI64S => Unary(U::F32ConvertI64S),
        Operator::F32ConvertI64U => Unary(U::F32ConvertI64U),
        Operator::F32DemoteF64 => Unary(U::F32DemoteF64),
        Operator::F64ConvertI32S => Unary(U::F64ConvertI32S),
        Operator::F64ConvertI32U => Unary(U::F64ConvertI32U),
        Operator::F64ConvertI64S => Unary(U::F64ConvertI64S),
        Operator::F64ConvertI64U => Unary(U::F64ConvertI64U),
        Operator::F64PromoteF32 => Unary(U::F64PromoteF32),
        Operator::I32ReinterpretF32 => Unary(U::I32ReinterpretF32),
        Operator::I64ReinterpretF64 => Unary(U::I64ReinterpretF64),
        Operator::F32ReinterpretI32 => Unary(U::F32ReinterpretI32),
        Operator::F64ReinterpretI64 => Unary(U::F64ReinterpretI64),
        Operator::I32Extend8S => Unary(U::I32Extend8S),
        Operator::I32Extend16S => Unary(U::I32Extend16S),
        Operator::I64Extend8S => Unary(U::I64Extend8S),
        Operator::I64Extend16S => Unary(U::I64Extend16S),
        Operator::I64Extend32S => Unary(U::I64Extend32S),
        Operator::I32TruncSatF32S => Unary(U::I32TruncSatF32S),
        Operator::I32TruncSatF32U => Unary(U::I32TruncSatF32U),
        Operator::I32TruncSatF64S => Unary(U::I32TruncSatF64S),
        Operator::I32TruncSatF64U => Unary(U::I32TruncSatF64U),
        Operator::I64TruncSatF32S => Unary(U::I64TruncSatF32S),
        Operator::I64TruncSatF32U => Unary(U::I64TruncSatF32U),
        Operator::I64TruncSatF64S => Unary(U::I64TruncSatF64S),
        Operator::I64TruncSatF64U => Unary(U::I64TruncSatF64U),

        Operator::I32Eq => Binary(B::I32Eq),
        Operator::I32Ne => Binary(B::I32Ne),
        Operator::I32LtS => Binary(B::I32LtS),
        Operator::I32LtU => Binary(B::I32LtU),
        Operator::I32GtS => Binary(B::I32GtS),
        Operator::I32GtU => Binary(B::I32GtU),
        Operator::I32LeS => Binary(B::I32LeS),
        Operator::I32LeU => Binary(B::I32LeU),
        Operator::I32GeS => Binary(B::I32GeS),
        Operator::I32GeU => Binary(B::I32GeU),
        Operator::I64Eq => Binary(B::I64Eq),
        Operator::I64Ne => Binary(B::I64Ne),
        Operator::I64LtS => Binary(B::I64LtS),
        Operator::I64LtU => Binary(B::I64LtU),
        Operator::I64GtS => Binary(B::I64GtS),
        Operator::I64GtU => Binary(B::I64GtU),
        Operator::I64LeS => Binary(B::I64LeS),
        Operator::I64LeU => Binary(B::I64LeU),
        Operator::I64GeS => Binary(B::I64GeS),
        Operator::I64GeU => Binary(B::I64GeU),
        Operator::F32Eq => Binary(B::F32Eq),
        Operator::F32Ne => Binary(B::F32Ne),
        Operator::F32Lt => Binary(B::F32Lt),
        Operator::F32Gt => Binary(B::F32Gt),
        Operator::F32Le => Binary(B::F32Le),
        Operator::F32Ge => Binary(B::F32Ge),
        Operator::F64Eq => Binary(B::F64Eq),
        Operator::F64Ne => Binary(B::F64Ne),
        Operator::F64Lt => Binary(B::F64Lt),
        Operator::F64Gt => Binary(B::F64Gt),
        Operator::F64Le => Binary(B::F64Le),
        Operator::F64Ge => Binary(B::F64Ge),
        Operator::I32Add => Binary(B::I32Add),
        Operator::I32Sub => Binary(B::I32Sub),
        Operator::I32Mul => Binary(B::I32Mul),
        Operator::I32DivS => Binary(B::I32DivS),
        Operator::I32DivU => Binary(B::I32DivU),
        Operator::I32RemS => Binary(B::I32RemS),
        Operator::I32RemU => Binary(B::I32RemU),
        Operator::I32And => Binary(B::I32And),
        Operator::I32Or => Binary(B::I32Or),
        Operator::I32Xor => Binary(B::I32Xor),
        Operator::I32Shl => Binary(B::I32Shl),
        Operator::I32ShrS => Binary(B::I32ShrS),
        Operator::I32ShrU => Binary(B::I32ShrU),
        Operator::I32Rotl => Binary(B::I32Rotl),
        Operator::I32Rotr => Binary(B::I32Rotr),
        Operator::I64Add => Binary(B::I64Add),
        Operator::I64Sub => Binary(B::I64Sub),
        Operator::I64Mul => Binary(B::I64Mul),
        Operator::I64DivS => Binary(B::I64DivS),
        Operator::I64DivU => Binary(B::I64DivU),
        Operator::I64RemS => Binary(B::I64RemS),
        Operator::I64RemU => Binary(B::I64RemU),
        Operator::I64And => Binary(B::I64And),
        Operator::I64Or => Binary(B::I64Or),
        Operator::I64Xor => Binary(B::I64Xor),
        Operator::I64Shl => Binary(B::I64Shl),
        Operator::I64ShrS => Binary(B::I64ShrS),
        Operator::I64ShrU => Binary(B::I64ShrU),
        Operator::I64Rotl => Binary(B::I64Rotl),
        Operator::I64Rotr => Binary(B::I64Rotr),
        Operator::F32Add => Binary(B::F32Add),
        Operator::F32Sub => Binary(B::F32Sub),
        Operator::F32Mul => Binary(B::F32Mul),
        Operator::F32Div => Binary(B::F32Div),
        Operator::F32Min => Binary(B::F32Min),
        Operator::F32Max => Binary(B::F32Max),
        Operator::F32Copysign => Binary(B::F32Copysign),
        Operator::F64Add => Binary(B::F64Add),
        Operator::F64Sub => Binary(B::F64Sub),
        Operator::F64Mul => Binary(B::F64Mul),
        Operator::F64Div => Binary(B::F64Div),
        Operator::F64Min => Binary(B::F64Min),
        Operator::F64Max => Binary(B::F64Max),
        Operator::F64Copysign => Binary(B::F64Copysign),

        Operator::V128Const { value } => {
            features.simd = true;
            Simd(SimdOp::Const(*value.bytes()))
        }
        Operator::I32x4Splat => {
            features.simd = true;
            Simd(SimdOp::I32x4Splat)
        }
        Operator::I32x4Add => {
            features.simd = true;
            Simd(SimdOp::I32x4Add)
        }
        Operator::I32x4Sub => {
            features.simd = true;
            Simd(SimdOp::I32x4Sub)
        }
        Operator::I32x4Mul => {
            features.simd = true;
            Simd(SimdOp::I32x4Mul)
        }
        Operator::I32x4Shl => {
            features.simd = true;
            Simd(SimdOp::I32x4Shl)
        }
        Operator::I32x4ExtractLane { lane } => {
            features.simd = true;
            Simd(SimdOp::I32x4Extract(lane))
        }
        Operator::I32x4ReplaceLane { lane } => {
            features.simd = true;
            Simd(SimdOp::I32x4Replace(lane))
        }
        Operator::F32x4Splat => {
            features.simd = true;
            Simd(SimdOp::F32x4Splat)
        }
        Operator::F32x4Add => {
            features.simd = true;
            Simd(SimdOp::F32x4Add)
        }
        Operator::F32x4Sub => {
            features.simd = true;
            Simd(SimdOp::F32x4Sub)
        }
        Operator::F32x4Mul => {
            features.simd = true;
            Simd(SimdOp::F32x4Mul)
        }
        Operator::F32x4Div => {
            features.simd = true;
            Simd(SimdOp::F32x4Div)
        }
        Operator::F32x4Abs => {
            features.simd = true;
            Simd(SimdOp::F32x4Abs)
        }
        Operator::F32x4Neg => {
            features.simd = true;
            Simd(SimdOp::F32x4Neg)
        }
        Operator::F32x4Sqrt => {
            features.simd = true;
            Simd(SimdOp::F32x4Sqrt)
        }
        Operator::F32x4ExtractLane { lane } => {
            features.simd = true;
            Simd(SimdOp::F32x4Extract(lane))
        }
        Operator::F32x4ReplaceLane { lane } => {
            features.simd = true;
            Simd(SimdOp::F32x4Replace(lane))
        }
        Operator::I8x16Shuffle { lanes } => {
            features.simd = true;
            Simd(SimdOp::I8x16Shuffle(lanes))
        }
        Operator::V128And => {
            features.simd = true;
            Simd(SimdOp::V128And)
        }
        Operator::V128Or => {
            features.simd = true;
            Simd(SimdOp::V128Or)
        }
        Operator::V128Xor => {
            features.simd = true;
            Simd(SimdOp::V128Xor)
        }
        Operator::V128Not => {
            features.simd = true;
            Simd(SimdOp::V128Not)
        }
        Operator::V128Bitselect => {
            features.simd = true;
            Simd(SimdOp::V128Bitselect)
        }
        Operator::V128AnyTrue => {
            features.simd = true;
            Simd(SimdOp::V128AnyTrue)
        }
        Operator::V128Load { memarg: m } => {
            features.simd = true;
            Simd(SimdOp::V128Load(memarg(m)))
        }
        Operator::V128Store { memarg: m } => {
            features.simd = true;
            Simd(SimdOp::V128Store(memarg(m)))
        }
        Operator::Throw { tag_index } => {
            features.exceptions = true;
            Throw(tag_index)
        }
        Operator::TryTable { try_table } => {
            features.exceptions = true;
            let catches = try_table
                .catches
                .into_iter()
                .map(|catch| match catch {
                    wasmparser::Catch::One { tag, label } => Ok(CatchClause::Tag { tag, label }),
                    wasmparser::Catch::All { label } => Ok(CatchClause::All { label }),
                    wasmparser::Catch::OneRef { .. } | wasmparser::Catch::AllRef { .. } => {
                        Err(CompileError::unsupported(
                            "exception references",
                            None,
                            "exception reference identity has no portable asm.js representation",
                        ))
                    }
                })
                .collect::<Result<Vec<_>, _>>()?;
            TryTable {
                sig: sig(try_table.ty, types)?,
                catches,
            }
        }

        other => {
            let instruction = format!("{other:?}");
            if instruction.contains("Atomic")
                || instruction.contains("Fence")
                || instruction.contains("Wait")
                || instruction.contains("Notify")
            {
                features.threads = true;
            } else if instruction.starts_with("Struct")
                || instruction.starts_with("Array")
                || instruction.contains("I31")
                || instruction.contains("Extern")
            {
                features.gc = true;
            } else if instruction.contains("V128")
                || instruction.contains("x16")
                || instruction.contains("x8")
                || instruction.contains("x4")
                || instruction.contains("x2")
            {
                features.simd = true;
            } else if instruction.contains("Throw")
                || instruction.contains("Catch")
                || instruction.contains("Try")
                || instruction.contains("Rethrow")
                || instruction.contains("Delegate")
            {
                features.exceptions = true;
            } else if instruction.contains("CallRef")
                || instruction.contains("RefCast")
                || instruction.contains("RefTest")
            {
                features.typed_references = true;
            } else {
                features.unsupported.push(instruction.clone());
            }
            Unsupported {
                feature: "unsupported instruction",
                instruction,
            }
        }
    };
    Ok(op)
}
