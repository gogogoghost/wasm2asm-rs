use crate::diagnostics::{CompileError, ErrorKind};
use crate::ir::*;
use crate::options::{CompileOptions, OutputFormat};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

#[cfg(test)]
#[path = "emitter/tests.rs"]
mod tests;

pub fn emit(module: &Module, options: &CompileOptions) -> Result<Vec<u8>, CompileError> {
    validate_boundaries(module)?;
    let mut cx = ModuleCx::new(module, options)?;
    let bare = cx.emit_module()?;
    let output = format_output(bare, options)?.into_bytes();
    if options.limits.max_js_bytes != 0 && output.len() > options.limits.max_js_bytes {
        return Err(CompileError::limit(
            "generated JavaScript bytes",
            output.len(),
            options.limits.max_js_bytes,
        ));
    }
    Ok(output)
}

fn format_output(mut bare: String, options: &CompileOptions) -> Result<String, CompileError> {
    match options.output_format {
        OutputFormat::Bare => {
            bare.push('\n');
            Ok(bare)
        }
        OutputFormat::EsModule => {
            bare.push_str("\nexport { instantiate };\nexport default instantiate;\n");
            Ok(bare)
        }
        OutputFormat::Umd => {
            if options.global_name.is_empty() {
                return Err(CompileError::new(
                    ErrorKind::InvalidInput,
                    "wasm2asm: --global-name must not be empty for UMD output",
                ));
            }
            Ok(format!(
                "(function(root,factory){{if(typeof module=='object'&&module.exports){{module.exports=factory();}}else{{root[{}]=factory();}}}})(typeof self!='undefined'?self:this,function(){{{}return{{instantiate:instantiate,'default':instantiate}};}});\n",
                js_string(&options.global_name),
                bare
            ))
        }
    }
}

fn validate_boundaries(module: &Module) -> Result<(), CompileError> {
    for export in &module.exports {
        match export.kind {
            ExportKind::Table => {
                return Err(CompileError::unsupported(
                    "exported tables",
                    None,
                    "JavaScript table mutation cannot carry canonical WebAssembly signatures",
                ));
            }
            ExportKind::Tag => {
                return Err(CompileError::unsupported(
                    "exported exception tags",
                    None,
                    "exception tags are module-private",
                ));
            }
            ExportKind::Func => {
                let ty = module
                    .function_type_indices
                    .get(export.index as usize)
                    .and_then(|i| module.types.get(*i as usize))
                    .ok_or_else(|| {
                        CompileError::new(
                            ErrorKind::Internal,
                            "wasm2asm: invalid exported function type",
                        )
                    })?;
                if ty.params.contains(&ValType::V128) || ty.results.contains(&ValType::V128) {
                    return Err(CompileError::unsupported(
                        "SIMD host boundary",
                        None,
                        "the asm.js host ABI has no v128 representation",
                    ));
                }
                if ty
                    .params
                    .iter()
                    .chain(&ty.results)
                    .any(|t| matches!(t, ValType::FuncRef(_)))
                {
                    return Err(CompileError::unsupported(
                        "function-reference host boundary",
                        None,
                        "the asm.js host ABI has no stable function-reference representation",
                    ));
                }
            }
            ExportKind::Global => {
                let ty = module
                    .global_types
                    .get(export.index as usize)
                    .ok_or_else(|| {
                        CompileError::new(ErrorKind::Internal, "wasm2asm: invalid exported global")
                    })?
                    .ty;
                if matches!(ty, ValType::V128 | ValType::FuncRef(_)) {
                    return Err(CompileError::unsupported(
                        "global host boundary",
                        None,
                        "v128 and reference globals cannot cross the JavaScript boundary",
                    ));
                }
            }
            ExportKind::Memory => {}
        }
    }
    for import in &module.imports {
        match import.kind {
            ImportKind::Func(type_index) => {
                let ty = &module.types[type_index as usize];
                if ty.results.len() > 1 {
                    return Err(CompileError::unsupported(
                        "imported multivalue function",
                        None,
                        "strict asm.js FFI exposes one return value; split the callback into scalar imports",
                    ));
                }
                if ty
                    .params
                    .iter()
                    .chain(&ty.results)
                    .any(|ty| matches!(ty, ValType::V128 | ValType::FuncRef(_)))
                {
                    return Err(CompileError::unsupported(
                        "function import host boundary",
                        None,
                        "v128 and function references cannot cross the asm.js FFI boundary",
                    ));
                }
            }
            ImportKind::Global(ty) if matches!(ty.ty, ValType::V128 | ValType::FuncRef(_)) => {
                return Err(CompileError::unsupported(
                    "global import host boundary",
                    None,
                    "v128 and reference globals cannot cross the JavaScript boundary",
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

fn mark_live_function(index: u32, live: &mut [bool], work: &mut Vec<usize>) {
    let Some(slot) = live.get_mut(index as usize) else {
        return;
    };
    if !*slot {
        *slot = true;
        work.push(index as usize);
    }
}

fn reachable_functions(module: &Module) -> (Vec<bool>, BTreeSet<u32>) {
    let mut live = vec![false; module.function_type_indices.len()];
    let mut work = Vec::new();

    for export in &module.exports {
        if export.kind == ExportKind::Func {
            mark_live_function(export.index, &mut live, &mut work);
        }
    }
    if let Some(start) = module.start {
        mark_live_function(start, &mut live, &mut work);
    }
    for element in &module.elements {
        for &item in &element.items {
            if let Some(index) = item {
                mark_live_function(index, &mut live, &mut work);
            }
        }
    }
    for global in &module.globals {
        if let ConstExpr::RefFunc(index) = global.init {
            mark_live_function(index, &mut live, &mut work);
        }
    }

    let mut indirect_types = BTreeSet::new();
    while let Some(function_index) = work.pop() {
        if function_index < module.imported_function_count as usize {
            continue;
        }
        let defined_index = function_index - module.imported_function_count as usize;
        let Some(function) = module.functions.get(defined_index) else {
            continue;
        };
        for instruction in &function.body {
            match &instruction.op {
                Op::Call(target) | Op::ReturnCall(target) | Op::RefFunc(target) => {
                    mark_live_function(*target, &mut live, &mut work);
                }
                Op::CallIndirect { type_index, .. }
                | Op::ReturnCallIndirect { type_index, .. }
                | Op::CallRef(type_index) => {
                    indirect_types.insert(*type_index);
                }
                _ => {}
            }
        }
    }

    (live, indirect_types)
}

type OffsetHelpers = BTreeMap<(u64, u8), String>;

fn memory_offset_helpers(
    module: &Module,
    live_functions: &[bool],
) -> (OffsetHelpers, OffsetHelpers) {
    if module.memories.len() != 1 || module.memories[0].memory64 {
        return (BTreeMap::new(), BTreeMap::new());
    }
    let mut loads = BTreeMap::<(u64, u8), usize>::new();
    let mut stores = BTreeMap::<(u64, u8), usize>::new();
    for (defined, function) in module.functions.iter().enumerate() {
        let index = module.imported_function_count as usize + defined;
        if !live_functions[index] {
            continue;
        }
        for instruction in &function.body {
            match instruction.op {
                Op::Load(op, arg) if arg.memory == 0 && arg.offset != 0 => {
                    *loads.entry((arg.offset, load_code(op))).or_default() += 1;
                }
                Op::Store(op, arg) if arg.memory == 0 && arg.offset != 0 => {
                    *stores.entry((arg.offset, store_code(op))).or_default() += 1;
                }
                _ => {}
            }
        }
    }
    let mut loads = loads
        .into_iter()
        .filter(|(_, count)| *count >= 16)
        .collect::<Vec<_>>();
    loads.sort_by(|(left_key, left_count), (right_key, right_count)| {
        right_count.cmp(left_count).then(left_key.cmp(right_key))
    });
    let loads = loads
        .into_iter()
        .enumerate()
        .map(|(index, (key, _))| (key, format!("$l{}", short_index(index))))
        .collect();
    let mut stores = stores
        .into_iter()
        .filter(|(_, count)| *count >= 16)
        .collect::<Vec<_>>();
    stores.sort_by(|(left_key, left_count), (right_key, right_count)| {
        right_count.cmp(left_count).then(left_key.cmp(right_key))
    });
    let stores = stores
        .into_iter()
        .enumerate()
        .map(|(index, (key, _))| (key, format!("$s{}", short_index(index))))
        .collect();
    (loads, stores)
}

fn direct_memory_size(module: &Module, live_functions: &[bool]) -> Option<u32> {
    let [memory] = module.memories.as_slice() else {
        return None;
    };
    if memory.memory64
        || memory.shared
        || memory.page_size_log2 != 16
        || module
            .exports
            .iter()
            .any(|export| export.kind == ExportKind::Memory)
    {
        return None;
    }
    let grows = module
        .functions
        .iter()
        .enumerate()
        .any(|(defined, function)| {
            let index = module.imported_function_count as usize + defined;
            live_functions[index]
                && function
                    .body
                    .iter()
                    .any(|instruction| matches!(&instruction.op, Op::MemoryGrow(_)))
        });
    if grows {
        return None;
    }
    let bytes = memory.initial.checked_shl(memory.page_size_log2)?;
    let conventional_asm_heap =
        bytes.is_power_of_two() || bytes >= 16 * 1024 * 1024 && bytes % (16 * 1024 * 1024) == 0;
    if bytes < 16 * 1024 * 1024 || bytes > 2 * 1024 * 1024 * 1024 || !conventional_asm_heap {
        return None;
    }
    u32::try_from(bytes).ok()
}

fn uses_inline_i64_helpers(module: &Module, live_functions: &[bool]) -> bool {
    module
        .functions
        .iter()
        .enumerate()
        .any(|(defined, function)| {
            let index = module.imported_function_count as usize + defined;
            live_functions[index]
                && function.body.iter().any(|instruction| {
                    matches!(
                        &instruction.op,
                        Op::Binary(
                            BinaryOp::I64Add
                                | BinaryOp::I64Sub
                                | BinaryOp::I64Mul
                                | BinaryOp::I64And
                                | BinaryOp::I64Or
                                | BinaryOp::I64Xor
                                | BinaryOp::I64Shl
                                | BinaryOp::I64ShrS
                                | BinaryOp::I64ShrU
                        )
                    )
                })
        })
}

struct ModuleCx<'a> {
    module: &'a Module,
    options: &'a CompileOptions,
    function_names: Vec<String>,
    import_function_names: Vec<String>,
    global_names: Vec<Value>,
    return_slots: Vec<Value>,
    return_slot_counts: [usize; 3],
    indirect_types: BTreeSet<u32>,
    live_functions: Vec<bool>,
    load_offset_helpers: OffsetHelpers,
    store_offset_helpers: OffsetHelpers,
    direct_memory_size: Option<u32>,
    inline_i64_helpers: bool,
}

impl<'a> ModuleCx<'a> {
    fn new(module: &'a Module, options: &'a CompileOptions) -> Result<Self, CompileError> {
        let mut function_names = Vec::with_capacity(module.function_type_indices.len());
        for index in 0..module.function_type_indices.len() {
            function_names.push(function_ident(index));
        }
        let import_function_names =
            function_names[..module.imported_function_count as usize].to_vec();
        let mut global_names = Vec::with_capacity(module.global_types.len());
        for (index, ty) in module.global_types.iter().enumerate() {
            global_names.push(named_value(ty.ty, &format!("$g{}", short_index(index))));
        }
        let (live_functions, indirect_types) = reachable_functions(module);
        let (load_offset_helpers, store_offset_helpers) =
            memory_offset_helpers(module, &live_functions);
        let direct_memory_size = direct_memory_size(module, &live_functions);
        let inline_i64_helpers = uses_inline_i64_helpers(module, &live_functions);
        let (return_slot_counts, return_slots) = return_slot_layout(module);
        Ok(Self {
            module,
            options,
            function_names,
            import_function_names,
            global_names,
            return_slots,
            return_slot_counts,
            indirect_types,
            live_functions,
            load_offset_helpers,
            store_offset_helpers,
            direct_memory_size,
            inline_i64_helpers,
        })
    }

    fn return_slot_indices(&self, results: &[ValType]) -> Vec<usize> {
        return_slot_indices(results, self.return_slot_counts)
    }

    fn return_slots_for(&self, results: &[ValType]) -> Vec<Value> {
        self.return_slot_indices(results)
            .into_iter()
            .map(|index| self.return_slots[index].clone())
            .collect()
    }

    fn emit_module(&mut self) -> Result<String, CompileError> {
        let mut out = String::new();
        self.emit_core(&mut out)?;
        out = out.replace(";}", "}");
        self.emit_wrapper_prefix(&mut out)?;
        self.emit_wrapper_suffix(&mut out)?;
        Ok(rename_module_functions(&out, &self.function_names))
    }

    fn emit_wrapper_prefix(&self, out: &mut String) -> Result<(), CompileError> {
        out.push_str("function instantiate(i){i=i||{};");
        out.push_str("var f={},r={a:[],s:[],m:[],p:[],d:[],e:[],t:[],u:[],z:[]},b,j,k,q=[],n=0,M=Math,F=M.fround,U=M.imul,C=M.clz32,B,V,H,hi=0,a=r.a,s=r.s,m=r.m,p=r.p,T=r.t,S=r.u,E=r.e,D=r.d,Z=r.z;");

        let mut function_import = 0usize;
        let mut memory_import = 0usize;
        let mut global_import = 0usize;
        for import in &self.module.imports {
            let access = format!(
                "(i[{}]||{{}})[{}]",
                js_string(&import.module),
                js_string(&import.name)
            );
            match import.kind {
                ImportKind::Func(_) => {
                    let name = &self.import_function_names[function_import];
                    write!(out, "f.{name}={access};if(typeof f.{name}!='function')throw TypeError('wasm2asm: import is not a function');").unwrap();
                    function_import += 1;
                }
                ImportKind::Memory(_) => {
                    write!(out, "r.i{memory_import}={access};").unwrap();
                    memory_import += 1;
                }
                ImportKind::Global(ty) => {
                    let suffix = short_index(global_import);
                    if ty.ty == ValType::I64 {
                        write!(out, "j={access};if(!j||typeof j!='object')throw TypeError('wasm2asm: i64 global import requires low/high');f.g{suffix}=j.low|0;f.h{suffix}=j.high|0;").unwrap();
                    } else {
                        write!(
                            out,
                            "j={access};f.g{suffix}=j&&typeof j=='object'&&'value'in j?j.value:j;"
                        )
                        .unwrap();
                    }
                    global_import += 1;
                }
                ImportKind::Table(_) => {
                    return Err(CompileError::unsupported(
                        "imported tables",
                        None,
                        "host tables do not expose WebAssembly signatures",
                    ));
                }
                ImportKind::Tag(_) => {
                    return Err(CompileError::unsupported(
                        "imported exception tags",
                        None,
                        "exception lowering requires module-private tags",
                    ));
                }
            }
        }
        if self.module.function_type_indices[..self.module.imported_function_count as usize]
            .iter()
            .any(|index| self.module.types[*index as usize].results.first() == Some(&ValType::I64))
        {
            out.push_str("f.h=((i.env||{}).getTempRet0)||function(){return 0};");
        }

        let mut base = 0u64;
        for (index, memory) in self.module.memories.iter().enumerate() {
            let page = 1u64 << memory.page_size_log2;
            let size = memory.initial.checked_mul(page).ok_or_else(|| {
                CompileError::limit(
                    "initial memory bytes",
                    u64::MAX,
                    self.options.limits.max_memory_pages.saturating_mul(65536),
                )
            })?;
            let max = memory
                .maximum
                .and_then(|v| v.checked_mul(page))
                .unwrap_or(-1i64 as u64);
            write!(
                out,
                "r.a[{index}]={base};r.s[{index}]={size};r.m[{index}]={};r.p[{index}]={page};",
                if max == u64::MAX {
                    "-1".into()
                } else {
                    max.to_string()
                }
            )
            .unwrap();
            base = base
                .checked_add(size)
                .ok_or_else(|| CompileError::limit("combined memory bytes", u64::MAX, u32::MAX))?;
        }
        if base > u32::MAX as u64 {
            return Err(CompileError::unsupported(
                "combined memories",
                None,
                "initial memories exceed the 32-bit asm.js heap",
            ));
        }
        if self.module.imported_memory_count == 1 && self.module.memories.len() == 1 {
            out.push_str("b=r.i0&&r.i0.buffer?r.i0.buffer:null;");
            write!(out, "if(!b||b.byteLength<{})throw RangeError('wasm2asm: imported memory is too small');", base).unwrap();
            out.push_str("r.b=b;");
        } else if !self.module.memories.is_empty() {
            write!(out, "b=new ArrayBuffer({base});r.b=b;").unwrap();
            for index in 0..self.module.imported_memory_count as usize {
                write!(out, "j=r.i{index};if(!j||!j.buffer)throw TypeError('wasm2asm: memory import is invalid');new Uint8Array(b,r.a[{index}],r.s[{index}]).set(new Uint8Array(j.buffer,0,Math.min(j.buffer.byteLength,r.s[{index}])));").unwrap();
            }
        } else {
            out.push_str("r.b=new ArrayBuffer(0);");
        }

        for (index, segment) in self.module.data.iter().enumerate() {
            write!(
                out,
                "r.d[{index}]={{b:{},n:{},x:0}};",
                js_byte_string(&segment.bytes),
                segment.bytes.len()
            )
            .unwrap();
        }
        for (index, element) in self.module.elements.iter().enumerate() {
            write!(out, "r.e[{index}]=[").unwrap();
            for (position, item) in element.items.iter().enumerate() {
                if position != 0 {
                    out.push(',');
                }
                write!(out, "{}", item.map(|v| v + 1).unwrap_or(0)).unwrap();
            }
            out.push_str("];r.e[");
            write!(out, "{index}").unwrap();
            out.push_str("].x=0;");
        }
        if let Some(table) = self.module.tables.first() {
            write!(
                out,
                "T.length={};S.length={};for(k=0;k<{};k++)T[k]=S[k]=0;r.l={};",
                table.initial,
                table.initial,
                table.initial,
                table
                    .maximum
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "-1".into())
            )
            .unwrap();
        } else {
            out.push_str("r.l=0;");
        }
        for (function_index, type_index) in self.module.function_type_indices.iter().enumerate() {
            if !self.live_functions[function_index] {
                continue;
            }
            write!(out, "Z[{}]={type_index};", function_index + 1).unwrap();
        }
        out.push_str("B=r.b;V=new DataView(B);H=new Uint8Array(B);");
        out.push_str(RUNTIME_HELPERS_PREFIX);
        out.push_str(if self.options.preserve_traps {
            CHECKED_ADDRESS_HELPER
        } else {
            FAST_ADDRESS_HELPER
        });
        out.push_str(RUNTIME_HELPERS_SUFFIX);
        out.push_str(if self.options.preserve_traps {
            CHECKED_INDIRECT_HELPER
        } else {
            FAST_INDIRECT_HELPER
        });
        out.push_str("f.X=X;f.ct=ct;f.pc=pc;f.tr=tr;f.ne=ne;f.mn=mn;f.mx=mx;f.cs=cs;f.rf=rf;f.ri=ri;f.rd=rd;f.wr=wr;f.Y=Y;f.y=y;f.W=W;f.GH=function(){return hi|0};f.AA=AA;f.LI=LI;f.l=l;f.LF=LF;f.lf=lf;f.SI=SI;f.st=st;f.SF=SF;f.sf=sf;f.VL=VL;f.vl=vl;f.VS=VS;f.vs=vs;f.MS=MS;f.DD=DD;f.ED=ED;f.TS=TS;f.RS=RS;f.IG=IG;f.G=G;f.AB=AB;f.AC=AC;f.K=K;f.N=N;f.O=O;f.P=P;f.Q=Q;f.R=R;f.L=L;");
        self.emit_outer_initializers(out)?;
        if self.direct_memory_size.is_some() {
            out.push_str("var x=asmModule({Math:M,NaN:NaN,Infinity:Infinity,Int8Array:Int8Array,Uint8Array:Uint8Array,Int16Array:Int16Array,Uint16Array:Uint16Array,Int32Array:Int32Array,Uint32Array:Uint32Array,Float32Array:Float32Array,Float64Array:Float64Array},f,B);");
        } else {
            out.push_str("var x=asmModule({Math:M,NaN:NaN,Infinity:Infinity},f);");
        }
        if self.module.start.is_some() {
            out.push_str("x.$start();delete x.$start;");
        }
        out.push_str("q=[");
        for index in 0..self.return_slots.len() {
            if index != 0 {
                out.push(',');
            }
            write!(out, "x.$q{}", short_index(index)).unwrap();
        }
        out.push_str("];");
        Ok(())
    }

    fn emit_wrapper_suffix(&self, out: &mut String) -> Result<(), CompileError> {
        for (export_position, export) in self.module.exports.iter().enumerate() {
            match export.kind {
                ExportKind::Func => {
                    let ty_index = self.module.function_type_indices[export.index as usize];
                    let ty = &self.module.types[ty_index as usize];
                    let key = js_string(&export.name);
                    write!(
                        out,
                        "j=x.e{};delete x.e{};",
                        short_index(export_position),
                        short_index(export_position)
                    )
                    .unwrap();
                    if ty.results.len() > 1 {
                        let slot_indices = self.return_slot_indices(&ty.results);
                        write!(out, "x[{key}]=(function(h){{return function(){{var a=h.apply(null,arguments),v=[").unwrap();
                        emit_primary_export_value(
                            out,
                            ty.results.first().copied(),
                            "a",
                            slot_indices.first().copied(),
                        );
                        let mut slot = if matches!(ty.results.first(), Some(ValType::I64)) {
                            1
                        } else {
                            0
                        };
                        for result in ty.results.iter().skip(1) {
                            out.push(',');
                            match result {
                                ValType::I64 => {
                                    write!(
                                        out,
                                        "[q[{}](),q[{}]() ]",
                                        slot_indices[slot],
                                        slot_indices[slot + 1]
                                    )
                                    .unwrap();
                                    slot += 2;
                                }
                                _ => {
                                    write!(out, "q[{}]()", slot_indices[slot]).unwrap();
                                    slot += 1;
                                }
                            }
                        }
                        out.push_str("];return v}})(j);");
                    } else {
                        write!(out, "x[{key}]=j;").unwrap();
                    }
                }
                ExportKind::Global => {
                    let key = js_string(&export.name);
                    let suffix = short_index(export.index as usize);
                    if self.module.global_types[export.index as usize].ty == ValType::I64 {
                        write!(out, "j={{}};Object.defineProperty(j,'low',{{enumerable:true,get:x.$g{suffix},").unwrap();
                        if self.module.global_types[export.index as usize].mutable {
                            write!(out, "set:x.$s{suffix}").unwrap();
                        } else {
                            out.push_str("set:function(){throw TypeError('immutable global')}");
                        }
                        write!(
                            out,
                            "}});Object.defineProperty(j,'high',{{enumerable:true,get:x.$h{suffix},"
                        )
                        .unwrap();
                        if self.module.global_types[export.index as usize].mutable {
                            write!(out, "set:x.$t{suffix}").unwrap();
                        } else {
                            out.push_str("set:function(){throw TypeError('immutable global')}");
                        }
                        write!(out, "}});x[{key}]=j;").unwrap();
                    } else {
                        write!(out, "j={{}};Object.defineProperty(j,'value',{{enumerable:true,get:x.$g{suffix},").unwrap();
                        if self.module.global_types[export.index as usize].mutable {
                            write!(out, "set:x.$s{suffix}").unwrap();
                        } else {
                            out.push_str("set:function(){throw TypeError('immutable global')}");
                        }
                        write!(out, "}});x[{key}]=j;").unwrap();
                    }
                }
                ExportKind::Memory => {
                    let key = js_string(&export.name);
                    write!(out, "j={{grow:x.$m{}}};Object.defineProperty(j,'buffer',{{enumerable:true,get:function(){{return r.b}}}});x[{key}]=j;", short_index(export.index as usize)).unwrap();
                }
                ExportKind::Table | ExportKind::Tag => {}
            }
        }
        if self.module.features.i64 && !self.return_slots.is_empty() {
            out.push_str("x.getTempRet0=q[0];");
        }
        for index in 0..self.return_slots.len() {
            write!(out, "delete x.$q{};", short_index(index)).unwrap();
        }
        for index in 0..self.module.global_types.len() {
            write!(
                out,
                "delete x.$g{};delete x.$s{};",
                short_index(index),
                short_index(index)
            )
            .unwrap();
            if self.module.global_types[index].ty == ValType::I64 {
                write!(
                    out,
                    "delete x.$h{};delete x.$t{};",
                    short_index(index),
                    short_index(index)
                )
                .unwrap();
            }
        }
        for index in 0..self.module.memories.len() {
            write!(out, "delete x.$m{};", short_index(index)).unwrap();
        }
        out.push_str("return x}");
        Ok(())
    }

    fn emit_core(&mut self, out: &mut String) -> Result<(), CompileError> {
        out.push_str("function asmModule(stdlib,foreign");
        if self.direct_memory_size.is_some() {
            out.push_str(",heap");
        }
        out.push_str("){\'use asm\';var F=stdlib.Math.fround,U=stdlib.Math.imul,C=stdlib.Math.clz32,Ma=stdlib.Math.abs,Mc=stdlib.Math.ceil,Mf=stdlib.Math.floor,Ms=stdlib.Math.sqrt,Na=stdlib.NaN,In=stdlib.Infinity,X=foreign.X,ct=foreign.ct,pc=foreign.pc,tr=foreign.tr,ne=foreign.ne,mn=foreign.mn,mx=foreign.mx,cs=foreign.cs,rf=foreign.rf,ri=foreign.ri,rd=foreign.rd,wr=foreign.wr,Y=foreign.Y,y=foreign.y,W=foreign.W,GH=foreign.GH,AA=foreign.AA,LI=foreign.LI,l=foreign.l,LF=foreign.LF,lf=foreign.lf,SI=foreign.SI,st=foreign.st,SF=foreign.SF,sf=foreign.sf,VL=foreign.VL,vl=foreign.vl,VS=foreign.VS,vs=foreign.vs,MS=foreign.MS,DD=foreign.DD,ED=foreign.ED,TS=foreign.TS,RS=foreign.RS,IG=foreign.IG,G=foreign.G,AB=foreign.AB,AC=foreign.AC,K=foreign.K,N=foreign.N,O=foreign.O,P=foreign.P,Q=foreign.Q,R=foreign.R,L=foreign.L,");
        if self.direct_memory_size.is_some() {
            out.push_str("$h8=new stdlib.Int8Array(heap),$u8=new stdlib.Uint8Array(heap),$h16=new stdlib.Int16Array(heap),$u16=new stdlib.Uint16Array(heap),$h32=new stdlib.Int32Array(heap),$u32=new stdlib.Uint32Array(heap),$f32=new stdlib.Float32Array(heap),$f64=new stdlib.Float64Array(heap),$ih=0,$rl=0,$rh=0,");
        } else if self.inline_i64_helpers {
            out.push_str("$ih=0,");
        }
        if self.return_slots.is_empty() {
            out.push_str("$q0=0;");
        } else {
            for (index, slot) in self.return_slots.iter().enumerate() {
                if index != 0 {
                    out.push(',');
                }
                let name = slot.components()[0];
                write!(out, "{name}={}", zero_literal(slot.ty())).unwrap();
            }
            out.push(';');
        }
        self.emit_import_aliases(out);
        self.emit_globals(out)?;
        if self.module.memories.iter().any(|memory| !memory.memory64) {
            if self.options.preserve_traps {
                out.push_str("function B0(a,o){a=a|0;o=o|0;if((a>>>0)>=((-o)>>>0))X();return (a+o)|0}function l2(a,o,t){a=a|0;o=o|0;t=t|0;return l(B0(a,o)|0,t|0)|0}function lf2(a,o,t){a=a|0;o=o|0;t=t|0;return +lf(B0(a,o)|0,t|0)}function st2(a,o,t,v,w){a=a|0;o=o|0;t=t|0;v=v|0;w=w|0;st(B0(a,o)|0,t|0,v|0,w|0)}function sf2(a,o,t,v){a=a|0;o=o|0;t=t|0;v=+v;sf(B0(a,o)|0,t|0,+v)}");
            } else {
                out.push_str("function B0(a,o){a=a|0;o=o|0;return (a+o)|0}function l2(a,o,t){a=a|0;o=o|0;t=t|0;return l(B0(a,o)|0,t|0)|0}function lf2(a,o,t){a=a|0;o=o|0;t=t|0;return +lf(B0(a,o)|0,t|0)}function st2(a,o,t,v,w){a=a|0;o=o|0;t=t|0;v=v|0;w=w|0;st(B0(a,o)|0,t|0,v|0,w|0)}function sf2(a,o,t,v){a=a|0;o=o|0;t=t|0;v=+v;sf(B0(a,o)|0,t|0,+v)}");
            }
        }
        self.emit_i64_helpers(out);
        self.emit_direct_memory_helpers(out);
        self.emit_memory_offset_helpers(out);
        self.emit_dispatchers(out)?;
        let mut compiled = Vec::new();
        for (defined_index, function) in self.module.functions.iter().enumerate() {
            let function_index = self.module.imported_function_count as usize + defined_index;
            if !self.live_functions[function_index] {
                continue;
            }
            let compiler = FunctionCompiler::new(self, function_index, function)?;
            compiled.push(CompiledFunction {
                index: function_index,
                code: compiler.compile()?,
            });
        }
        let static_memory_helpers = optimize_static_memory_accesses(&mut compiled);
        out.push_str(&static_memory_helpers);
        self.emit_compiled_functions(out, &compiled)?;
        self.emit_export_object(out)?;
        out.push('}');
        Ok(())
    }

    fn emit_i64_helpers(&self, out: &mut String) {
        if !self.inline_i64_helpers {
            return;
        }
        out.push_str("function $m(a,b,c,d){a=a|0;b=b|0;c=c|0;d=d|0;var w0=0,t=0,w1=0,w2=0,h=0;w0=U(a&65535,c&65535)|0;t=U(a>>>16,c&65535)+(w0>>>16)|0;w0=w0&65535;w1=t&65535;w2=t>>>16;w1=U(a&65535,c>>>16)+w1|0;h=U(a>>>16,c>>>16)+w2|0;h=h+(w1>>>16)|0;h=h+U(b,c)|0;h=h+U(a,d)|0;return h|0}");
        out.push_str("function $g(a,b,c,d){a=a|0;b=b|0;c=c|0;d=d|0;var n=0;n=c&63;if(!n){$ih=b;return a|0}if((n|0)<32){$ih=(b<<n)|(a>>>(32-n));return a<<n}$ih=a<<(n-32);return 0}");
        out.push_str("function $h(a,b,c,d){a=a|0;b=b|0;c=c|0;d=d|0;var n=0;n=c&63;if(!n){$ih=b;return a|0}if((n|0)<32){$ih=b>>n;return (a>>>n)|(b<<(32-n))}$ih=b>>31;return b>>(n-32)}");
        out.push_str("function $i(a,b,c,d){a=a|0;b=b|0;c=c|0;d=d|0;var n=0;n=c&63;if(!n){$ih=b;return a|0}if((n|0)<32){$ih=b>>>n;return (a>>>n)|(b<<(32-n))}$ih=0;return (b>>>(n-32))|0}");
    }

    fn emit_direct_memory_helpers(&self, out: &mut String) {
        let Some(size) = self.direct_memory_size else {
            return;
        };
        let byte_limit = size - 1;
        let half_limit = size - 2;
        let word_limit = size - 4;
        let double_limit = size - 8;
        let check = |limit| {
            self.options
                .preserve_traps
                .then(|| format!("if((a>>>0)>{limit})X();"))
                .unwrap_or_default()
        };
        let byte_check = check(byte_limit);
        let half_check = check(half_limit);
        let word_check = check(word_limit);
        let double_check = check(double_limit);
        write!(
            out,
            "function $L0(a){{a=a|0;{word_check}if(a&3)return l(a|0,0|0)|0;return $h32[a>>2]|0}}"
        )
        .unwrap();
        write!(out, "function $L1(a){{a=a|0;var v=0;{double_check}if(a&3){{v=l(a|0,1|0)|0;$ih=GH()|0;return v|0}}$ih=$h32[(a+4)>>2]|0;return $h32[a>>2]|0}}").unwrap();
        write!(
            out,
            "function $F2(a){{a=a|0;{word_check}if(a&3)return +lf(a|0,2|0);return +F($f32[a>>2])}}"
        )
        .unwrap();
        write!(out, "function $F3(a){{a=a|0;var v=0.0;{double_check}if(a&7){{v=+lf(a|0,3|0);$rl=l(a|0,1|0)|0;$rh=GH()|0;return +v}}$rl=$h32[a>>2]|0;$rh=$h32[(a+4)>>2]|0;return +$f64[a>>3]}}").unwrap();
        for code in [4, 8] {
            write!(
                out,
                "function $L{code}(a){{a=a|0;{byte_check}return $h8[a>>0]|0}}"
            )
            .unwrap();
        }
        for code in [5, 9] {
            write!(
                out,
                "function $L{code}(a){{a=a|0;{byte_check}return $u8[a>>0]|0}}"
            )
            .unwrap();
        }
        for code in [6, 10] {
            write!(out, "function $L{code}(a){{a=a|0;{half_check}if(a&1)return l(a|0,{code}|0)|0;return $h16[a>>1]|0}}").unwrap();
        }
        for code in [7, 11] {
            write!(out, "function $L{code}(a){{a=a|0;{half_check}if(a&1)return l(a|0,{code}|0)|0;return $u16[a>>1]|0}}").unwrap();
        }
        write!(
            out,
            "function $L12(a){{a=a|0;{word_check}if(a&3)return l(a|0,12|0)|0;return $h32[a>>2]|0}}"
        )
        .unwrap();
        write!(
            out,
            "function $L13(a){{a=a|0;{word_check}if(a&3)return l(a|0,13|0)|0;return $u32[a>>2]|0}}"
        )
        .unwrap();
        write!(out, "function $S0(a,v){{a=a|0;v=v|0;{word_check}if(a&3){{st(a|0,0|0,v|0,0|0);return}}$h32[a>>2]=v}}").unwrap();
        write!(out, "function $S1(a,v,w){{a=a|0;v=v|0;w=w|0;{double_check}if(a&3){{st(a|0,1|0,v|0,w|0);return}}$h32[a>>2]=v;$h32[(a+4)>>2]=w}}").unwrap();
        write!(out, "function $D2(a,v){{a=a|0;v=+v;{word_check}if(a&3){{sf(a|0,2|0,+v);return}}$f32[a>>2]=F(v)}}").unwrap();
        write!(out, "function $D3(a,v){{a=a|0;v=+v;{double_check}if(a&7){{sf(a|0,3|0,+v);return}}$f64[a>>3]=v}}").unwrap();
        for code in [4, 6] {
            write!(
                out,
                "function $S{code}(a,v){{a=a|0;v=v|0;{byte_check}$h8[a>>0]=v}}"
            )
            .unwrap();
        }
        for code in [5, 7] {
            write!(out, "function $S{code}(a,v){{a=a|0;v=v|0;{half_check}if(a&1){{st(a|0,{code}|0,v|0,0|0);return}}$h16[a>>1]=v}}").unwrap();
        }
        write!(out, "function $S8(a,v){{a=a|0;v=v|0;{word_check}if(a&3){{st(a|0,8|0,v|0,0|0);return}}$h32[a>>2]=v}}").unwrap();
    }

    fn emit_memory_offset_helpers(&self, out: &mut String) {
        if self.direct_memory_size.is_some() {
            for (&(offset, code), name) in &self.load_offset_helpers {
                if matches!(code, 2 | 3) {
                    write!(
                        out,
                        "function {name}(a){{a=a|0;return +$F{code}(B0(a,{offset})|0)}}"
                    )
                    .unwrap();
                } else {
                    write!(
                        out,
                        "function {name}(a){{a=a|0;return $L{code}(B0(a,{offset})|0)|0}}"
                    )
                    .unwrap();
                }
            }
            for (&(offset, code), name) in &self.store_offset_helpers {
                if matches!(code, 2 | 3) {
                    write!(
                        out,
                        "function {name}(a,v){{a=a|0;v=+v;$D{code}(B0(a,{offset})|0,+v)}}"
                    )
                    .unwrap();
                } else if code == 1 {
                    write!(
                        out,
                        "function {name}(a,v,w){{a=a|0;v=v|0;w=w|0;$S1(B0(a,{offset})|0,v|0,w|0)}}"
                    )
                    .unwrap();
                } else {
                    write!(
                        out,
                        "function {name}(a,v){{a=a|0;v=v|0;$S{code}(B0(a,{offset})|0,v|0)}}"
                    )
                    .unwrap();
                }
            }
            return;
        }
        for (&(offset, code), name) in &self.load_offset_helpers {
            if matches!(code, 2 | 3) {
                write!(
                    out,
                    "function {name}(a){{a=a|0;return +lf2(a,{offset},{code})}}"
                )
                .unwrap();
            } else {
                write!(
                    out,
                    "function {name}(a){{a=a|0;return l2(a,{offset},{code})|0}}"
                )
                .unwrap();
            }
        }
        for (&(offset, code), name) in &self.store_offset_helpers {
            if matches!(code, 2 | 3) {
                write!(
                    out,
                    "function {name}(a,v){{a=a|0;v=+v;sf2(a,{offset},{code},+v)}}"
                )
                .unwrap();
            } else if matches!(code, 1 | 6 | 7 | 8) {
                write!(
                    out,
                    "function {name}(a,v,w){{a=a|0;v=v|0;w=w|0;st2(a,{offset},{code},v,w)}}"
                )
                .unwrap();
            } else {
                write!(
                    out,
                    "function {name}(a,v){{a=a|0;v=v|0;st2(a,{offset},{code},v,0)}}"
                )
                .unwrap();
            }
        }
    }

    fn emit_compiled_functions(
        &self,
        out: &mut String,
        compiled: &[CompiledFunction],
    ) -> Result<(), CompileError> {
        let mut groups: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for (position, function) in compiled.iter().enumerate() {
            let body = compiled_function_body(&function.code).ok_or_else(|| {
                CompileError::new(ErrorKind::Internal, "wasm2asm: malformed compiled function")
            })?;
            let literals = numeric_literals(body);
            let type_index = self.module.function_type_indices[function.index];
            let key = format!("{type_index}:{}", numeric_template_key(body, &literals));
            groups.entry(key).or_default().push(position);
        }

        let mut templates = Vec::new();
        let mut helper_index = self.function_names.len();
        for members in groups.into_values() {
            if members.len() < 2 {
                continue;
            }
            let representative = &compiled[members[0]];
            let representative_body =
                compiled_function_body(&representative.code).ok_or_else(|| {
                    CompileError::new(ErrorKind::Internal, "wasm2asm: malformed compiled function")
                })?;
            let representative_literals = numeric_literals(representative_body);
            let member_bodies: Vec<_> = members
                .iter()
                .map(|&position| {
                    compiled_function_body(&compiled[position].code).ok_or_else(|| {
                        CompileError::new(
                            ErrorKind::Internal,
                            "wasm2asm: malformed compiled function",
                        )
                    })
                })
                .collect::<Result<_, CompileError>>()?;
            let member_literals: Vec<_> = member_bodies
                .iter()
                .map(|body| numeric_literals(body))
                .collect();
            if member_literals
                .iter()
                .any(|literals| literals.len() != representative_literals.len())
            {
                continue;
            }

            let differing: Vec<usize> = (0..representative_literals.len())
                .filter(|&index| {
                    !representative_literals[index].case_label
                        && !representative_literals[index].must_literal
                        && member_literals
                            .iter()
                            .zip(&member_bodies)
                            .any(|(literals, body)| {
                                literal_text(body, &literals[index])
                                    != literal_text(
                                        representative_body,
                                        &representative_literals[index],
                                    )
                            })
                })
                .collect();
            if differing.is_empty() {
                continue;
            }

            let helper = function_ident(helper_index);
            let helper_body = render_numeric_template_body(
                representative_body,
                &representative_literals,
                &differing,
            );
            let helper_code = self.render_template_helper(
                &helper,
                representative.index,
                &helper_body,
                differing.len(),
            )?;
            let mut wrappers = Vec::with_capacity(members.len());
            for (member_position, &position) in members.iter().enumerate() {
                let body = member_bodies[member_position];
                let literals = &member_literals[member_position];
                let constants = differing
                    .iter()
                    .map(|&index| literal_text(body, &literals[index]).to_string())
                    .collect::<Vec<_>>();
                wrappers.push(self.render_template_wrapper(
                    &helper,
                    compiled[position].index,
                    &constants,
                )?);
            }
            let original_size: usize = members
                .iter()
                .map(|&position| compiled[position].code.len())
                .sum();
            let replacement_size =
                helper_code.len() + wrappers.iter().map(String::len).sum::<usize>();
            if replacement_size >= original_size {
                continue;
            }
            templates.push(FunctionTemplate {
                members,
                helper: helper_code,
                wrappers,
            });
            helper_index += 1;
        }

        let mut by_position = BTreeMap::new();
        for (template_index, template) in templates.iter().enumerate() {
            for &position in &template.members {
                by_position.insert(position, template_index);
            }
        }
        for (position, function) in compiled.iter().enumerate() {
            if let Some(&template_index) = by_position.get(&position) {
                let template = &templates[template_index];
                if template.members[0] == position {
                    out.push_str(&template.helper);
                    for wrapper in &template.wrappers {
                        out.push_str(wrapper);
                    }
                }
            } else {
                out.push_str(&function.code);
            }
        }
        Ok(())
    }

    fn render_template_helper(
        &self,
        helper: &str,
        function_index: usize,
        body: &str,
        constant_count: usize,
    ) -> Result<String, CompileError> {
        let ty = self.function_type(function_index)?;
        let params = parameter_values(&ty.params);
        let mut names = flatten_names(&params);
        let constants: Vec<String> = (0..constant_count)
            .map(|index| format!("c{index}"))
            .collect();
        names.extend(constants.iter().cloned());
        let mut out = String::new();
        write!(out, "function {helper}({}){{", names.join(",")).unwrap();
        emit_param_coercions(&mut out, &params);
        for constant in &constants {
            write!(out, "{constant}={constant}|0;").unwrap();
        }
        let mut parameter_prefix = String::new();
        emit_param_coercions(&mut parameter_prefix, &params);
        out.push_str(body.strip_prefix(&parameter_prefix).unwrap_or(body));
        out.push('}');
        Ok(out)
    }

    fn render_template_wrapper(
        &self,
        helper: &str,
        function_index: usize,
        constants: &[String],
    ) -> Result<String, CompileError> {
        let ty = self.function_type(function_index)?;
        let params = parameter_values(&ty.params);
        let flat_params = flatten_names(&params);
        let mut args = flat_params.clone();
        args.extend(constants.iter().cloned());
        let call = format!("{helper}({})", args.join(","));
        let mut out = String::new();
        write!(
            out,
            "function {}({}){{",
            self.function_names[function_index],
            flat_params.join(",")
        )
        .unwrap();
        emit_param_coercions(&mut out, &params);
        if ty.results.is_empty() {
            write!(out, "{call};return;").unwrap();
        } else {
            emit_direct_js_return(&mut out, &ty.results, &call, false);
        }
        out.push('}');
        Ok(out)
    }

    fn emit_import_aliases(&self, out: &mut String) {
        for (function_index, name) in self.import_function_names.iter().enumerate() {
            if !self.live_functions[function_index] {
                continue;
            }
            write!(out, "var {name}=foreign.{name};").unwrap();
        }
        if self.module.function_type_indices[..self.module.imported_function_count as usize]
            .iter()
            .any(|index| self.module.types[*index as usize].results.first() == Some(&ValType::I64))
        {
            out.push_str("var gh=foreign.h;");
        }
    }

    fn emit_globals(&self, out: &mut String) -> Result<(), CompileError> {
        for index in 0..self.module.imported_global_count as usize {
            let suffix = short_index(index);
            match &self.global_names[index] {
                Value::I64(lo, hi) => write!(
                    out,
                    "var {lo}=foreign.g{suffix}|0,{hi}=foreign.h{suffix}|0;"
                )
                .unwrap(),
                value => emit_var_value(out, value, &format!("foreign.g{suffix}")),
            }
        }
        for (defined, global) in self.module.globals.iter().enumerate() {
            let index = self.module.imported_global_count as usize + defined;
            let value = match global.init {
                ConstExpr::GlobalGet(source) if source < self.module.imported_global_count => {
                    let suffix = short_index(source as usize);
                    match global.ty.ty {
                        ValType::I32 | ValType::FuncRef(_) => {
                            Value::I32(format!("foreign.g{suffix}|0"))
                        }
                        ValType::I64 => Value::I64(
                            format!("foreign.g{suffix}|0"),
                            format!("foreign.h{suffix}|0"),
                        ),
                        ValType::F32 => Value::F32(format!("F(foreign.g{suffix})")),
                        ValType::F64 => Value::F64(format!("+foreign.g{suffix}")),
                        ValType::V128 => {
                            unreachable!("SIMD globals cannot cross the host boundary")
                        }
                    }
                }
                _ => const_value(&global.init, &self.global_names)?,
            };
            emit_var_value_from_value(out, &self.global_names[index], &value);
        }
        Ok(())
    }

    fn emit_dispatchers(&self, out: &mut String) -> Result<(), CompileError> {
        for &type_index in &self.indirect_types {
            let ty = self.module.types.get(type_index as usize).ok_or_else(|| {
                CompileError::new(ErrorKind::Internal, "wasm2asm: invalid dispatcher type")
            })?;
            let params = parameter_values(&ty.params);
            let flat = flatten_names(&params);
            write!(out, "function I{}(x", short_index(type_index as usize)).unwrap();
            for name in &flat {
                write!(out, ",{name}").unwrap();
            }
            out.push_str("){x=x|0;");
            emit_param_coercions(out, &params);
            write!(out, "x=IG(x|0,{type_index}|0)|0;").unwrap();
            let call = format!(
                "J{}(x{})",
                short_index(type_index as usize),
                flat.iter()
                    .map(|name| format!(",{name}"))
                    .collect::<String>()
            );
            emit_direct_js_return(out, &ty.results, &call, false);
            out.push('}');

            write!(out, "function J{}(x", short_index(type_index as usize)).unwrap();
            for name in &flat {
                write!(out, ",{name}").unwrap();
            }
            out.push_str("){x=x|0;");
            emit_param_coercions(out, &params);
            let imported_i64 = ty.results.first() == Some(&ValType::I64)
                && self.module.function_type_indices
                    [..self.module.imported_function_count as usize]
                    .contains(&type_index);
            if imported_i64 {
                out.push_str("var y=0;");
            }
            out.push_str("if(!x)X();switch(x|0){");
            for (function_index, function_type) in
                self.module.function_type_indices.iter().enumerate()
            {
                if *function_type != type_index || !self.live_functions[function_index] {
                    continue;
                }
                write!(out, "case {}:", function_index + 1).unwrap();
                let arguments = flatten_call_arguments(
                    &params,
                    function_index < self.module.imported_function_count as usize,
                );
                let call = format!(
                    "{}({})",
                    self.function_names[function_index],
                    arguments.join(",")
                );
                if function_index < self.module.imported_function_count as usize
                    && ty.results.first() == Some(&ValType::I64)
                {
                    let slots = self.return_slots_for(&ty.results);
                    let slot = slots[0].components()[0];
                    write!(out, "y={call}|0;{slot}=(gh())|0;return y|0;").unwrap();
                } else {
                    emit_direct_js_return(
                        out,
                        &ty.results,
                        &call,
                        function_index < self.module.imported_function_count as usize,
                    );
                }
            }
            out.push_str("default:X()}");
            match ty.results.first() {
                None => {}
                Some(ValType::F32) => out.push_str("return F(0)"),
                Some(ValType::F64) => out.push_str("return +0"),
                _ => out.push_str("return 0"),
            }
            out.push('}');
        }
        Ok(())
    }

    fn emit_outer_initializers(&self, out: &mut String) -> Result<(), CompileError> {
        for (index, segment) in self.module.data.iter().enumerate() {
            if let DataMode::Active { memory, ref offset } = segment.mode {
                let address = self.outer_offset(offset)?;
                write!(
                    out,
                    "K({index}|0,{memory}|0,({address})|0,0,{}|0);",
                    segment.bytes.len()
                )
                .unwrap();
            }
        }
        for (index, segment) in self.module.elements.iter().enumerate() {
            if let ElementMode::Active { table, ref offset } = segment.mode {
                if table != 0 {
                    return Err(CompileError::unsupported(
                        "multiple tables",
                        None,
                        "only table zero is supported",
                    ));
                }
                let address = self.outer_offset(offset)?;
                write!(
                    out,
                    "L({index}|0,({address})|0,0,{}|0);",
                    segment.items.len()
                )
                .unwrap();
            }
        }
        Ok(())
    }

    fn outer_offset(&self, expr: &ConstExpr) -> Result<String, CompileError> {
        match *expr {
            ConstExpr::I32(value) => Ok(value.to_string()),
            ConstExpr::I64(value) => {
                let bits = value as u64;
                Ok(if bits >> 32 == 0 {
                    (bits as u32).to_string()
                } else {
                    "-1".into()
                })
            }
            ConstExpr::GlobalGet(index) if index < self.module.imported_global_count => {
                Ok(format!("f.g{}", short_index(index as usize)))
            }
            _ => Err(CompileError::unsupported(
                "segment offset",
                None,
                "strict asm.js initialization supports integer constants and imported immutable i32 globals",
            )),
        }
    }

    fn emit_export_object(&self, out: &mut String) -> Result<(), CompileError> {
        for (index, slot) in self.return_slots.iter().enumerate() {
            write!(out, "function $Q{}(){{", short_index(index)).unwrap();
            emit_return_value(out, slot);
            out.push('}');
        }
        for (index, value) in self.global_names.iter().enumerate() {
            let suffix = short_index(index);
            if let Value::I64(lo, hi) = value {
                write!(
                    out,
                    "function $G{suffix}(){{return {lo}|0}}function $H{suffix}(){{return {hi}|0}}"
                )
                .unwrap();
                if self.module.global_types[index].mutable {
                    write!(out, "function $S{suffix}(v){{v=v|0;{lo}=v|0}}function $T{suffix}(v){{v=v|0;{hi}=v|0}}").unwrap();
                }
            } else {
                write!(out, "function $G{suffix}(){{").unwrap();
                emit_return_value(out, value);
                out.push('}');
                if self.module.global_types[index].mutable {
                    let ty = self.module.global_types[index].ty;
                    write!(out, "function $S{suffix}(v){{v={};", coerce(ty, "v")).unwrap();
                    emit_set_from_single_argument(out, value, "v");
                    out.push('}');
                }
            }
        }
        for index in 0..self.module.memories.len() {
            write!(
                out,
                "function $M{}(n){{n=n|0;return G({index}|0,n|0)|0}}",
                short_index(index)
            )
            .unwrap();
        }

        out.push_str("return{");
        let mut first = true;
        for (export_position, export) in self.module.exports.iter().enumerate() {
            if export.kind != ExportKind::Func {
                continue;
            }
            if !first {
                out.push(',');
            }
            first = false;
            write!(
                out,
                "e{}:{}",
                short_index(export_position),
                self.function_names[export.index as usize]
            )
            .unwrap();
        }
        if let Some(start) = self.module.start {
            if !first {
                out.push(',');
            }
            first = false;
            write!(out, "$start:{}", self.function_names[start as usize]).unwrap();
        }
        for index in 0..self.return_slots.len() {
            if !first {
                out.push(',');
            }
            first = false;
            write!(out, "$q{}:$Q{}", short_index(index), short_index(index)).unwrap();
        }
        for index in 0..self.global_names.len() {
            if !first {
                out.push(',');
            }
            first = false;
            let suffix = short_index(index);
            write!(out, "$g{suffix}:$G{suffix}").unwrap();
            if self.module.global_types[index].ty == ValType::I64 {
                write!(out, ",$h{suffix}:$H{suffix}").unwrap();
            }
            if self.module.global_types[index].mutable {
                write!(out, ",$s{suffix}:$S{suffix}").unwrap();
                if self.module.global_types[index].ty == ValType::I64 {
                    write!(out, ",$t{suffix}:$T{suffix}").unwrap();
                }
            }
        }
        for index in 0..self.module.memories.len() {
            if !first {
                out.push(',');
            }
            first = false;
            write!(out, "$m{}:$M{}", short_index(index), short_index(index)).unwrap();
        }
        out.push_str("};");
        Ok(())
    }

    fn function_type(&self, index: usize) -> Result<&FuncType, CompileError> {
        let type_index = *self
            .module
            .function_type_indices
            .get(index)
            .ok_or_else(|| {
                CompileError::new(ErrorKind::Internal, "wasm2asm: invalid function index")
            })?;
        self.module.types.get(type_index as usize).ok_or_else(|| {
            CompileError::new(ErrorKind::Internal, "wasm2asm: invalid function type index")
        })
    }
}

#[derive(Debug, Clone)]
enum Value {
    I32(String),
    I64(String, String),
    F32(String),
    F64(String),
    Ref(String),
    V128([String; 4]),
}

impl Value {
    fn ty(&self) -> ValType {
        match self {
            Self::I32(_) => ValType::I32,
            Self::I64(_, _) => ValType::I64,
            Self::F32(_) => ValType::F32,
            Self::F64(_) => ValType::F64,
            Self::Ref(_) => ValType::FuncRef(None),
            Self::V128(_) => ValType::V128,
        }
    }
    fn components(&self) -> Vec<&str> {
        match self {
            Self::I32(a) | Self::F32(a) | Self::F64(a) | Self::Ref(a) => vec![a],
            Self::I64(a, b) => vec![a, b],
            Self::V128(a) => a.iter().map(String::as_str).collect(),
        }
    }
    fn i32_expr(&self) -> Result<&str, CompileError> {
        match self {
            Self::I32(v) | Self::Ref(v) => Ok(v),
            _ => Err(CompileError::new(
                ErrorKind::Internal,
                "wasm2asm: expected i32 value",
            )),
        }
    }
    fn mentions_identifier(&self, name: &str) -> bool {
        match self {
            Self::I32(value) | Self::F32(value) | Self::F64(value) | Self::Ref(value) => {
                expression_mentions_identifier(value, name)
            }
            Self::I64(low, high) => {
                expression_mentions_identifier(low, name)
                    || expression_mentions_identifier(high, name)
            }
            Self::V128(values) => values
                .iter()
                .any(|value| expression_mentions_identifier(value, name)),
        }
    }
}

fn expression_mentions_identifier(expression: &str, name: &str) -> bool {
    expression.match_indices(name).any(|(offset, _)| {
        let bytes = expression.as_bytes();
        let before = offset.checked_sub(1).and_then(|index| bytes.get(index));
        let after = bytes.get(offset + name.len());
        before.is_none_or(|byte| !is_js_identifier_byte(*byte))
            && after.is_none_or(|byte| !is_js_identifier_byte(*byte))
    })
}

fn is_js_identifier_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'$')
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ControlKind {
    Block,
    Loop,
    If,
}

#[derive(Debug, Clone)]
struct Control {
    kind: ControlKind,
    label: String,
    label_start: usize,
    label_used: bool,
    params: Vec<Value>,
    preserved_stack: Vec<Value>,
    results: Vec<ValType>,
    result_values: Vec<Value>,
    entry_reachable: bool,
    end_reachable: bool,
    then_reachable: bool,
    seen_else: bool,
}

struct CompiledFunction {
    index: usize,
    code: String,
}
struct FunctionTemplate {
    members: Vec<usize>,
    helper: String,
    wrappers: Vec<String>,
}
#[derive(Clone)]
struct NumericLiteral {
    start: usize,
    end: usize,
    case_label: bool,
    must_literal: bool,
}

struct FunctionCompiler<'a, 'm> {
    module: &'a ModuleCx<'m>,
    function_index: usize,
    function: &'a Function,
    ty: &'a FuncType,
    locals: Vec<Value>,
    stack: Vec<Value>,
    controls: Vec<Control>,
    // Pool temporaries across instructions; the per-instruction set protects Rust-local values.
    temp_slots: Vec<(String, ValType)>,
    instruction_temps: Vec<usize>,
    declarations: Vec<(String, ValType)>,
    f64_bits: BTreeMap<String, (String, String)>,
    body: String,
    temp_index: usize,
    label_index: usize,
    reachable: bool,
}

impl<'a, 'm> FunctionCompiler<'a, 'm> {
    fn new(
        module: &'a ModuleCx<'m>,
        function_index: usize,
        function: &'a Function,
    ) -> Result<Self, CompileError> {
        let ty = module.function_type(function_index)?;
        let mut locals = parameter_values(&ty.params);
        let mut next_local = flatten_names(&locals).len();
        for &local in &function.locals {
            let value = named_value(local, &local_ident(next_local));
            next_local += value.components().len();
            locals.push(value);
        }
        Ok(Self {
            module,
            function_index,
            function,
            ty,
            locals,
            stack: Vec::new(),
            controls: Vec::new(),
            declarations: Vec::new(),
            temp_slots: Vec::new(),
            instruction_temps: Vec::new(),
            f64_bits: BTreeMap::new(),
            body: String::new(),
            temp_index: next_local,
            label_index: 0,
            reachable: true,
        })
    }

    fn compile(mut self) -> Result<String, CompileError> {
        for instruction in &self.function.body {
            self.emit_instruction(instruction)?;
        }
        self.body = eliminate_write_only_i64_high_multiply(&self.body);
        let name = &self.module.function_names[self.function_index];
        let params = &self.locals[..self.ty.params.len()];
        let flat_params = flatten_names(params);
        let mut out = String::new();
        write!(out, "function {name}({}){{", flat_params.join(",")).unwrap();
        emit_param_coercions(&mut out, params);
        let param_components: usize = params.iter().map(|v| v.components().len()).sum();
        let all_components = flatten_names(&self.locals);
        let local_components = &all_components[param_components..];
        let live_local_components = local_components
            .iter()
            .filter(|component| expression_mentions_identifier(&self.body, component))
            .collect::<Vec<_>>();
        let live_declarations = self
            .declarations
            .iter()
            .filter(|(name, _)| expression_mentions_identifier(&self.body, name))
            .collect::<Vec<_>>();
        let mut has_declarations = false;
        for component in &live_local_components {
            if has_declarations {
                out.push(',');
            } else {
                out.push_str("var ");
                has_declarations = true;
            }
            let ty = component_type(&self.locals, component).unwrap_or(ValType::I32);
            write!(out, "{component}={}", zero_literal(ty)).unwrap();
        }
        for (name, ty) in &live_declarations {
            if has_declarations {
                out.push(',');
            } else {
                out.push_str("var ");
                has_declarations = true;
            }
            write!(out, "{name}={}", zero_literal(*ty)).unwrap();
        }
        if has_declarations {
            out.push(';');
        }
        out.push_str(&self.body);
        if self.ty.results.is_empty() {
            if self.reachable {
                out.push_str("return;");
            }
        } else {
            let tail = self.body.trim_end().trim_end_matches(';');
            let final_fragment = tail
                .rsplit([';', '{', '}'])
                .next()
                .unwrap_or("")
                .trim_start();
            if !final_fragment.starts_with("return ") {
                match self.ty.results[0] {
                    ValType::F32 => out.push_str("return F(0);"),
                    ValType::F64 => out.push_str("return +0;"),
                    _ => out.push_str("return 0;"),
                }
            }
        }
        out.push('}');
        let mut identifiers = live_local_components
            .into_iter()
            .map(|name| (*name).clone())
            .collect::<Vec<_>>();
        identifiers.extend(live_declarations.into_iter().map(|(name, _)| name.clone()));
        let out = rename_function_locals(&out, &identifiers, &flat_params);
        let out = rename_function_labels(&out);
        Ok(optimize_labeled_early_exits(&out))
    }

    fn emit_instruction(&mut self, instruction: &Instr) -> Result<(), CompileError> {
        use Op::*;
        self.instruction_temps.clear();
        if !self.reachable
            && !matches!(
                instruction.op,
                Block(_) | Loop(_) | If(_) | TryTable { .. } | Else | End
            )
        {
            return Ok(());
        }
        match &instruction.op {
            Unreachable => {
                self.body.push_str("X();");
                self.reachable = false;
            }
            Nop => {}
            Block(sig) => self.begin_control(ControlKind::Block, sig.clone())?,
            Loop(sig) => self.begin_control(ControlKind::Loop, sig.clone())?,
            If(sig) => {
                let condition = if self.reachable {
                    compact_condition(&self.pop_i32()?)
                } else {
                    "0".into()
                };
                self.begin_control_with_condition(ControlKind::If, sig.clone(), condition)?;
            }
            TryTable { .. } | Throw(_) => {
                return Err(self.internal("exception operation reached the asm.js emitter"));
            }
            Else => self.emit_else()?,
            End => self.end_control()?,
            Br(depth) => self.emit_br(*depth, true)?,
            BrIf(depth) => self.emit_br_if(*depth)?,
            BrTable { targets, default } => self.emit_br_table(targets, *default)?,
            Return => self.emit_function_return(true)?,
            Call(index) => self.emit_call(*index, false)?,
            ReturnCall(index) => self.emit_call(*index, true)?,
            CallIndirect {
                type_index,
                table_index,
            } => self.emit_indirect(*type_index, *table_index, false, false)?,
            ReturnCallIndirect {
                type_index,
                table_index,
            } => self.emit_indirect(*type_index, *table_index, true, false)?,
            CallRef(type_index) => self.emit_indirect(*type_index, 0, false, true)?,
            GlobalGet(index) => {
                let value = self
                    .module
                    .global_names
                    .get(*index as usize)
                    .ok_or_else(|| self.internal("invalid global index"))?
                    .clone();
                if self
                    .module
                    .module
                    .global_types
                    .get(*index as usize)
                    .is_some_and(|global| global.mutable)
                {
                    let snapshot = self.materialize(&value);
                    self.stack.push(snapshot);
                } else {
                    self.stack.push(value);
                }
            }
            Drop => {
                self.pop()?;
            }
            Select(_) => self.emit_select()?,
            LocalGet(index) => self.stack.push(self.local(*index)?.clone()),
            LocalSet(index) => {
                let value = self.pop()?;
                let target = self.local(*index)?.clone();
                let before_preserve = self.body.len();
                self.preserve_local_values(&target);
                let value = self.preserve_assignment_source(&target, value);
                if self.body.len() != before_preserve
                    || !self.retarget_last_temp_assignment(&target, &value)
                {
                    self.assign(&target, &value);
                }
            }
            LocalTee(index) => {
                let value = self.pop()?;
                let target = self.local(*index)?.clone();
                let before_preserve = self.body.len();
                self.preserve_local_values(&target);
                let value = self.preserve_assignment_source(&target, value);
                if self.body.len() != before_preserve
                    || !self.retarget_last_temp_assignment(&target, &value)
                {
                    self.assign(&target, &value);
                }
                self.stack.push(target);
            }
            GlobalSet(index) => {
                let value = self.pop()?;
                let target = self
                    .module
                    .global_names
                    .get(*index as usize)
                    .ok_or_else(|| self.internal("invalid global index"))?
                    .clone();
                let value = self.preserve_assignment_source(&target, value);
                if !self.retarget_last_temp_assignment(&target, &value) {
                    self.assign(&target, &value);
                }
            }
            I32Const(value) => self.stack.push(Value::I32(value.to_string())),
            I64Const(value) => self.stack.push(i64_const(*value)),
            F32Const(bits) => self.stack.push(Value::F32(float32_literal(*bits))),
            F64Const(bits) => {
                const EXPONENT: u64 = 0x7ff0_0000_0000_0000;
                const MANTISSA: u64 = 0x000f_ffff_ffff_ffff;
                if bits & EXPONENT == EXPONENT && bits & MANTISSA != 0 {
                    let low = (*bits as u32 as i32).to_string();
                    let high = ((*bits >> 32) as u32 as i32).to_string();
                    let expression = format!("+wr({low},{high})");
                    self.f64_bits.insert(expression.clone(), (low, high));
                    self.stack.push(Value::F64(expression));
                } else {
                    self.stack.push(Value::F64(float64_literal(*bits)));
                }
            }
            Unary(op) => self.emit_unary(*op)?,
            Binary(op) => self.emit_binary(*op)?,
            Load(op, memarg) => self.emit_load(*op, *memarg)?,
            Store(op, memarg) => self.emit_store(*op, *memarg)?,
            MemorySize(memory) => {
                let temp = self.temp(ValType::I32);
                write!(self.body, "{}=MS({memory}|0)|0;", temp.i32_expr()?).unwrap();
                if self.module.module.memories[*memory as usize].memory64 {
                    self.stack
                        .push(Value::I64(temp.i32_expr()?.into(), "0".into()));
                } else {
                    self.stack.push(temp);
                }
            }
            MemoryGrow(memory) => {
                if self.module.module.memories[*memory as usize].memory64 {
                    let (delta_lo, delta_hi) = expect_i64(self.pop()?)?;
                    let result_lo = self.temp(ValType::I32);
                    let result_hi = self.temp(ValType::I32);
                    write!(self.body, "if(({delta_hi})|0){{{}=-1;{}=-1;}}else{{{}=G({memory}|0,({delta_lo})|0)|0;{}=({})>>31;}}", result_lo.i32_expr()?, result_hi.i32_expr()?, result_lo.i32_expr()?, result_hi.i32_expr()?, result_lo.i32_expr()?).unwrap();
                    self.stack.push(Value::I64(
                        result_lo.i32_expr()?.into(),
                        result_hi.i32_expr()?.into(),
                    ));
                } else {
                    let delta = self.pop_i32()?.to_string();
                    let temp = self.temp(ValType::I32);
                    write!(
                        self.body,
                        "{}=G({memory}|0,({delta})|0)|0;",
                        temp.i32_expr()?
                    )
                    .unwrap();
                    self.stack.push(temp);
                }
            }
            MemoryCopy { dst, src } => self.emit_memory_copy(*dst, *src)?,
            MemoryFill(memory) => self.emit_memory_fill(*memory)?,
            MemoryInit { data, memory } => self.emit_memory_init(*data, *memory)?,
            DataDrop(index) => self.body.push_str(&format!("DD({index}|0);")),
            TableGet(table) => {
                self.ensure_table_zero(*table)?;
                let index = self.pop_table_index(*table)?;
                let temp = self.temp(ValType::FuncRef(None));
                write!(self.body, "{}=N(({index})|0)|0;", temp.i32_expr()?).unwrap();
                self.stack.push(temp);
            }
            TableSet(table) => {
                self.ensure_table_zero(*table)?;
                let value = self.pop()?;
                let index = self.pop_table_index(*table)?;
                write!(self.body, "O(({index})|0,({})|0);", value.i32_expr()?).unwrap();
            }
            TableSize(table) => {
                self.ensure_table_zero(*table)?;
                let temp = self.temp(ValType::I32);
                write!(self.body, "{}=TS()|0;", temp.i32_expr()?).unwrap();
                if self.module.module.tables[*table as usize].table64 {
                    self.stack
                        .push(Value::I64(temp.i32_expr()?.into(), "0".into()));
                } else {
                    self.stack.push(temp);
                }
            }
            TableGrow(table) => {
                self.ensure_table_zero(*table)?;
                if self.module.module.tables[*table as usize].table64 {
                    let (delta_lo, delta_hi) = expect_i64(self.pop()?)?;
                    let value = self.pop()?;
                    let result_lo = self.temp(ValType::I32);
                    let result_hi = self.temp(ValType::I32);
                    write!(self.body, "if(({delta_hi})|0){{{}=-1;{}=-1;}}else{{{}=P(({})|0,({delta_lo})|0)|0;{}=({})>>31;}}", result_lo.i32_expr()?, result_hi.i32_expr()?, result_lo.i32_expr()?, value.i32_expr()?, result_hi.i32_expr()?, result_lo.i32_expr()?).unwrap();
                    self.stack.push(Value::I64(
                        result_lo.i32_expr()?.into(),
                        result_hi.i32_expr()?.into(),
                    ));
                } else {
                    let delta = self.pop_i32()?.to_string();
                    let value = self.pop()?;
                    let temp = self.temp(ValType::I32);
                    write!(
                        self.body,
                        "{}=P(({})|0,({delta})|0)|0;",
                        temp.i32_expr()?,
                        value.i32_expr()?
                    )
                    .unwrap();
                    self.stack.push(temp);
                }
            }
            TableFill(table) => {
                self.ensure_table_zero(*table)?;
                let count = self.pop_table_index(*table)?;
                let value = self.pop()?;
                let dst = self.pop_table_index(*table)?;
                write!(
                    self.body,
                    "Q(({dst})|0,({})|0,({count})|0);",
                    value.i32_expr()?
                )
                .unwrap();
            }
            TableCopy { dst, src } => {
                self.ensure_table_zero(*dst)?;
                self.ensure_table_zero(*src)?;
                let count = self.pop_table_index(*dst)?;
                let source = self.pop_table_index(*src)?;
                let dest = self.pop_table_index(*dst)?;
                write!(self.body, "R(({dest})|0,({source})|0,({count})|0);").unwrap();
            }
            TableInit { elem, table } => {
                self.ensure_table_zero(*table)?;
                let count = self.pop_table_index(*table)?;
                let source = self.pop_table_index(*table)?;
                let dest = self.pop_table_index(*table)?;
                write!(
                    self.body,
                    "L({elem}|0,({dest})|0,({source})|0,({count})|0);"
                )
                .unwrap();
            }
            ElemDrop(index) => self.body.push_str(&format!("ED({index}|0);")),
            RefNull(_) => self.stack.push(Value::Ref("0".into())),
            RefIsNull => {
                let value = self.pop()?;
                self.stack
                    .push(Value::I32(format!("(({})|0)==0|0", value.i32_expr()?)));
            }
            RefFunc(index) => self.stack.push(Value::Ref((index + 1).to_string())),
            RefTest(type_index) => {
                let value = self.pop()?;
                let check = match type_index {
                    Some(t) => format!("RS(({})|0,{t}|0)|0", value.i32_expr()?),
                    None => format!("((({})|0)!=0)|0", value.i32_expr()?),
                };
                self.stack.push(Value::I32(check));
            }
            RefCast(type_index) => {
                let value = self.pop()?;
                if let Some(t) = type_index {
                    self.body.push_str(&format!(
                        "if((RS(({})|0,{t}|0)|0)==0)X();",
                        value.i32_expr()?
                    ));
                }
                self.stack.push(value);
            }
            Simd(op) => self.emit_simd(op)?,
            Unsupported {
                feature,
                instruction,
            } => {
                return Err(CompileError::unsupported(
                    feature,
                    Some(instruction_offset(instruction, instruction)),
                    format!("instruction {instruction}"),
                ));
            }
        }
        Ok(())
    }

    fn begin_control(&mut self, kind: ControlKind, sig: BlockSig) -> Result<(), CompileError> {
        self.begin_control_impl(kind, sig, None)
    }

    fn begin_control_with_condition(
        &mut self,
        kind: ControlKind,
        sig: BlockSig,
        condition: String,
    ) -> Result<(), CompileError> {
        self.begin_control_impl(kind, sig, Some(condition))
    }

    fn begin_control_impl(
        &mut self,
        kind: ControlKind,
        sig: BlockSig,
        condition: Option<String>,
    ) -> Result<(), CompileError> {
        let params = if self.reachable {
            self.pop_types(&sig.params)?
        } else {
            sig.params.iter().map(|&t| self.temp(t)).collect()
        };
        if self.reachable {
            // Values below the block parameters survive the control body. Snapshot them before
            // nested instructions can mutate locals referenced by their deferred expressions.
            for index in 0..self.stack.len() {
                let value = self.stack[index].clone();
                self.stack[index] = self.materialize(&value);
            }
        }
        let preserved_stack = self.stack.clone();
        let stored_params: Vec<Value> = sig.params.iter().map(|&t| self.temp(t)).collect();
        if self.reachable {
            self.assign_values(&stored_params, &params);
        }
        self.stack.extend(stored_params.iter().cloned());
        let result_values = sig.results.iter().map(|&t| self.temp(t)).collect();
        let label = compact_index(self.label_index);
        self.label_index += 1;
        let label_start = self.body.len();
        match kind {
            ControlKind::Block => write!(self.body, "{label}:{{").unwrap(),
            ControlKind::Loop => write!(self.body, "{label}:for(;;){{").unwrap(),
            ControlKind::If => write!(
                self.body,
                "{label}:if{}{{",
                condition_syntax(condition.as_deref().unwrap_or("0"))
            )
            .unwrap(),
        }
        self.controls.push(Control {
            kind,
            label,
            label_start,
            label_used: false,
            preserved_stack,
            params: stored_params,
            results: sig.results,
            result_values,
            entry_reachable: self.reachable,
            end_reachable: false,
            then_reachable: false,
            seen_else: false,
        });
        Ok(())
    }

    fn emit_else(&mut self) -> Result<(), CompileError> {
        let index = self
            .controls
            .len()
            .checked_sub(1)
            .ok_or_else(|| self.internal("else without control frame"))?;
        if self.controls[index].kind != ControlKind::If {
            return Err(self.internal("else outside if"));
        }
        if self.reachable {
            let values = self.pop_types(&self.controls[index].results.clone())?;
            let targets = self.controls[index].result_values.clone();
            self.assign_values(&targets, &values);
        }
        self.controls[index].then_reachable = self.reachable;
        self.controls[index].seen_else = true;
        self.body.push_str("}else{");
        self.stack = self.controls[index].preserved_stack.clone();
        self.stack.extend(self.controls[index].params.clone());
        self.reachable = self.controls[index].entry_reachable;
        Ok(())
    }

    fn end_control(&mut self) -> Result<(), CompileError> {
        if self.controls.is_empty() {
            if self.reachable {
                self.emit_function_return(false)?;
            }
            return Ok(());
        }
        let mut control = self.controls.pop().unwrap();
        if self.reachable {
            let values = self.pop_types(&control.results)?;
            self.assign_values(&control.result_values, &values);
        }
        match control.kind {
            ControlKind::Loop => {
                if self.reachable {
                    self.body.push_str("break;");
                }
                self.body.push('}');
            }
            _ => self.body.push('}'),
        }
        if !control.label_used {
            self.body.replace_range(
                control.label_start..control.label_start + control.label.len() + 1,
                "",
            );
        }
        let after = match control.kind {
            ControlKind::If if control.seen_else => {
                control.then_reachable || self.reachable || control.end_reachable
            }
            ControlKind::If => control.entry_reachable || self.reachable || control.end_reachable,
            _ => self.reachable || control.end_reachable,
        };
        self.stack = std::mem::take(&mut control.preserved_stack);
        if after {
            self.stack.append(&mut control.result_values);
        }
        self.reachable = after;
        Ok(())
    }

    fn emit_br(&mut self, depth: u32, consume: bool) -> Result<(), CompileError> {
        let target = self.target_index(depth)?;
        let types = self.branch_types(target);
        let values = if consume {
            self.pop_types(&types)?
        } else {
            self.peek_types(&types)?
        };
        self.emit_branch_to_index(target, &values)?;
        if consume {
            self.reachable = false;
        }
        Ok(())
    }

    fn emit_br_if(&mut self, depth: u32) -> Result<(), CompileError> {
        let condition = compact_condition(&self.pop_i32()?);
        let target = self.target_index(depth)?;
        let values = self.peek_types(&self.branch_types(target))?;
        let condition = condition_syntax(&condition);
        if values.is_empty() {
            write!(self.body, "if{condition}").unwrap();
            self.emit_branch_to_index(target, &values)?;
        } else {
            write!(self.body, "if{condition}{{").unwrap();
            self.emit_branch_to_index(target, &values)?;
            self.body.push('}');
        }
        Ok(())
    }

    fn emit_br_table(&mut self, targets: &[u32], default: u32) -> Result<(), CompileError> {
        let selector = compact_i32(&self.pop_i32()?.to_string());
        let default_target = self.target_index(default)?;
        let values = self.pop_types(&self.branch_types(default_target))?;
        let mut groups = Vec::<(usize, Vec<usize>)>::new();
        for (case, depth) in targets.iter().enumerate() {
            let target = self.target_index(*depth)?;
            if target == default_target {
                continue;
            }
            if let Some((_, cases)) = groups.iter_mut().find(|(group, _)| *group == target) {
                cases.push(case);
            } else {
                groups.push((target, vec![case]));
            }
        }
        write!(self.body, "switch({selector}){{").unwrap();
        for (target, cases) in groups {
            for case in cases {
                write!(self.body, "case {case}:").unwrap();
            }
            self.emit_branch_to_index(target, &values)?;
        }
        self.body.push_str("default:");
        self.emit_branch_to_index(default_target, &values)?;
        self.body.push('}');
        self.reachable = false;
        Ok(())
    }

    fn emit_branch_to_index(
        &mut self,
        target: usize,
        values: &[Value],
    ) -> Result<(), CompileError> {
        self.controls[target].label_used = true;
        let control = self.controls[target].clone();
        let targets = if control.kind == ControlKind::Loop {
            control.params
        } else {
            control.result_values
        };
        self.assign_values(&targets, values);
        if control.kind == ControlKind::Loop {
            write!(self.body, "continue {};", control.label).unwrap();
        } else {
            self.controls[target].end_reachable = true;
            write!(self.body, "break {};", control.label).unwrap();
        }
        Ok(())
    }

    fn emit_function_return(&mut self, consume: bool) -> Result<(), CompileError> {
        let results = self.ty.results.clone();
        let values = if consume {
            self.pop_types(&results)?
        } else if results.is_empty() {
            Vec::new()
        } else {
            self.pop_types(&results)?
        };
        let slots = self.module.return_slots_for(&results);
        if !self.inline_last_temp_return(&values) {
            emit_return_abi(&mut self.body, &values, &slots);
        }
        self.reachable = false;
        Ok(())
    }

    fn emit_call(&mut self, index: u32, tail: bool) -> Result<(), CompileError> {
        let ty = self.module.function_type(index as usize)?.clone();
        let args = self.pop_types(&ty.params)?;
        if tail
            && self.module.module.features.tail_call
            && index >= self.module.module.imported_function_count
        {
            write!(
                self.body,
                "throw{{w:2,f:{index},a:[{}]}};",
                flatten_names(&args).join(",")
            )
            .unwrap();
            self.reachable = false;
            return Ok(());
        }
        let arguments =
            flatten_call_arguments(&args, index < self.module.module.imported_function_count);
        let call = format!(
            "{}({})",
            self.module.function_names[index as usize],
            arguments.join(",")
        );
        self.finish_call(
            &ty.results,
            call,
            tail,
            index < self.module.module.imported_function_count,
        )
    }

    fn emit_indirect(
        &mut self,
        type_index: u32,
        table_index: u32,
        tail: bool,
        reference: bool,
    ) -> Result<(), CompileError> {
        if !reference {
            self.ensure_table_zero(table_index)?;
        }
        let ty = self
            .module
            .module
            .types
            .get(type_index as usize)
            .ok_or_else(|| self.internal("invalid indirect type"))?
            .clone();
        let callee = if reference {
            self.pop_i32()?.to_string()
        } else {
            self.pop_table_index(table_index)?
        };
        let args = self.pop_types(&ty.params)?;
        let flat = flatten_names(&args);
        if tail && self.module.module.features.tail_call {
            let target = self.temp(ValType::FuncRef(None));
            if reference {
                self.assign(&target, &Value::Ref(callee));
            } else {
                write!(self.body, "{}=N({callee})|0;", target.i32_expr()?).unwrap();
            }
            let target_name = target.i32_expr()?.to_string();
            write!(
                self.body,
                "if(!{target_name}||Z[{target_name}]!={type_index})X();"
            )
            .unwrap();
            if self.module.module.imported_function_count != 0 {
                write!(
                    self.body,
                    "if({target_name}<={}){{",
                    self.module.module.imported_function_count
                )
                .unwrap();
                let call = if flat.is_empty() {
                    format!("J{}({target_name})", short_index(type_index as usize))
                } else {
                    format!(
                        "J{}({target_name},{})",
                        short_index(type_index as usize),
                        flat.join(",")
                    )
                };
                self.finish_call(&ty.results, call, true, false)?;
                self.body.push('}');
                self.reachable = true;
            }
            write!(
                self.body,
                "throw{{w:2,f:({target_name}-1)|0,a:[{}]}};",
                flat.join(",")
            )
            .unwrap();
            self.reachable = false;
            return Ok(());
        }
        let function = if reference {
            format!("J{}", short_index(type_index as usize))
        } else {
            format!("I{}", short_index(type_index as usize))
        };
        let call = if flat.is_empty() {
            format!("{function}({callee})")
        } else {
            format!("{function}({callee},{})", flat.join(","))
        };
        self.finish_call(&ty.results, call, tail, false)
    }

    fn finish_call(
        &mut self,
        results: &[ValType],
        call: String,
        tail: bool,
        imported: bool,
    ) -> Result<(), CompileError> {
        if results.is_empty() {
            write!(self.body, "{call};").unwrap();
            if tail {
                self.body.push_str("return;");
                self.reachable = false;
            }
            return Ok(());
        }
        let primary = self.temp(match results[0] {
            ValType::I64 | ValType::V128 => ValType::I32,
            other => other,
        });
        let slots = self.module.return_slots_for(results);
        let values = call_result_values(results, &call, &slots, &mut self.body, primary, imported)?;
        for value in &values {
            if let Value::F64(expression) = value {
                let (low, high) = self.ensure_f64_bits(expression);
                write!(self.body, "{low}=rd(+({expression}))|0;{high}=GH()|0;").unwrap();
            }
        }
        if imported && results.first() == Some(&ValType::I64) {
            let slot = slots[0].components()[0];
            write!(self.body, "{slot}=(gh())|0;").unwrap();
        }
        if tail {
            if !self.inline_last_temp_return(&values) {
                emit_return_abi(&mut self.body, &values, &slots);
            }
            self.reachable = false;
        } else {
            self.stack.extend(values);
        }
        Ok(())
    }

    fn emit_select(&mut self) -> Result<(), CompileError> {
        let condition = self.pop_i32()?.to_string();
        let right = self.pop()?;
        let left = self.pop()?;
        if left.ty() != right.ty() {
            return Err(self.internal("select type mismatch"));
        }
        let condition = compact_operand(&condition);
        let value = match (left, right) {
            (Value::I32(a), Value::I32(b)) => Value::I32(format!(
                "({condition}?{}:{})|0",
                compact_operand(&a),
                compact_operand(&b)
            )),
            (Value::Ref(a), Value::Ref(b)) => Value::Ref(format!(
                "({condition}?{}:{})|0",
                compact_operand(&a),
                compact_operand(&b)
            )),
            (Value::F32(a), Value::F32(b)) => Value::F32(format!(
                "F({condition}?{}:{})",
                compact_operand(&a),
                compact_operand(&b)
            )),
            (Value::F64(a), Value::F64(b)) => {
                let left = Value::F64(a);
                let right = Value::F64(b);
                let result = self.temp(ValType::F64);
                write!(
                    self.body,
                    "if{}{{",
                    condition_syntax(&compact_i32(&condition))
                )
                .unwrap();
                self.assign(&result, &left);
                self.body.push_str("}else{");
                self.assign(&result, &right);
                self.body.push('}');
                result
            }
            (left, right) => {
                let result = self.temp(left.ty());
                write!(
                    self.body,
                    "if{}{{",
                    condition_syntax(&compact_i32(&condition))
                )
                .unwrap();
                self.assign(&result, &left);
                self.body.push_str("}else{");
                self.assign(&result, &right);
                self.body.push('}');
                result
            }
        };
        self.stack.push(value);
        Ok(())
    }

    fn emit_unary(&mut self, op: UnaryOp) -> Result<(), CompileError> {
        use UnaryOp::*;
        let value = self.pop()?;
        let result = match op {
            I32Eqz => Value::I32(format!(
                "{}==0|0",
                compact_compare_operand(value.i32_expr()?, false)
            )),
            I32Clz => Value::I32(format!("C({})", compact_i32(value.i32_expr()?))),
            I32Ctz => Value::I32(format!("ct({})|0", compact_i32(value.i32_expr()?))),
            I32Popcnt => Value::I32(format!("pc({})|0", compact_i32(value.i32_expr()?))),
            I64Eqz => {
                let (lo, hi) = expect_i64(value)?;
                Value::I32(format!("({}|{})==0|0", compact_i32(&lo), compact_i32(&hi)))
            }
            I64Clz => {
                let (lo, hi) = expect_i64(value)?;
                let result = self.temp(ValType::I32);
                let target = result.i32_expr()?;
                let hi = compact_i32(&hi);
                let lo = compact_i32(&lo);
                write!(
                    self.body,
                    "if({hi}){{{target}=C({hi})|0;}}else{{{target}=(32+(C({lo})|0))|0;}}"
                )
                .unwrap();
                Value::I64(target.into(), "0".into())
            }
            I64Ctz => {
                let (lo, hi) = expect_i64(value)?;
                let result = self.temp(ValType::I32);
                let target = result.i32_expr()?;
                let lo = compact_i32(&lo);
                let hi = compact_i32(&hi);
                write!(
                    self.body,
                    "if({lo}){{{target}=ct({lo})|0;}}else{{{target}=(32+(ct({hi})|0))|0;}}"
                )
                .unwrap();
                Value::I64(target.into(), "0".into())
            }
            I64Popcnt => {
                let (lo, hi) = expect_i64(value)?;
                Value::I64(
                    format!(
                        "((pc({})|0)+(pc({})|0))|0",
                        compact_i32(&lo),
                        compact_i32(&hi)
                    ),
                    "0".into(),
                )
            }
            F32Abs => Value::F32(format!("F(Ma({}))", expect_f32(value)?)),
            F32Neg => {
                let x = expect_f32(value)?;
                Value::F32(format!("F(-{})", compact_operand(&x)))
            }
            F32Ceil => Value::F32(format!("F(Mc({}))", expect_f32(value)?)),
            F32Floor => Value::F32(format!("F(Mf({}))", expect_f32(value)?)),
            F32Trunc => {
                let x = expect_f32(value)?;
                Value::F32(format!("F(+tr({}))", compact_float_argument(&x)))
            }
            F32Nearest => {
                let x = expect_f32(value)?;
                Value::F32(format!("F(+ne({}))", compact_float_argument(&x)))
            }
            F32Sqrt => Value::F32(format!("F(Ms({}))", expect_f32(value)?)),
            F64Abs => Value::F64(format!("+Ma({})", expect_f64(value)?)),
            F64Neg => {
                let x = expect_f64(value)?;
                Value::F64(format!("-{}", compact_operand(&x)))
            }
            F64Ceil => Value::F64(format!("+Mc({})", expect_f64(value)?)),
            F64Floor => Value::F64(format!("+Mf({})", expect_f64(value)?)),
            F64Trunc => {
                let x = expect_f64(value)?;
                Value::F64(format!("+tr({})", compact_float_argument(&x)))
            }
            F64Nearest => {
                let x = expect_f64(value)?;
                Value::F64(format!("+ne({})", compact_float_argument(&x)))
            }
            F64Sqrt => Value::F64(format!("+Ms({})", expect_f64(value)?)),
            I32WrapI64 => {
                let (lo, _) = expect_i64(value)?;
                Value::I32(lo)
            }
            I32TruncF32S | I32TruncF64S => {
                let x = expect_float(value)?;
                self.trunc_i32(x, false, false)?
            }
            I32TruncF32U | I32TruncF64U => {
                let x = expect_float(value)?;
                self.trunc_i32(x, true, false)?
            }
            I64ExtendI32S => {
                let x = compact_i32(value.i32_expr()?);
                Value::I64(x.clone(), format!("({x})>>31"))
            }
            I64ExtendI32U => {
                let x = compact_i32(value.i32_expr()?);
                Value::I64(x, "0".into())
            }
            I64TruncF32S | I64TruncF64S => {
                let x = expect_float(value)?;
                self.trunc_i64(x, false, false)?
            }
            I64TruncF32U | I64TruncF64U => {
                let x = expect_float(value)?;
                self.trunc_i64(x, true, false)?
            }
            F32ConvertI32S => Value::F32(format!("F({})", compact_i32(value.i32_expr()?))),
            F32ConvertI32U => Value::F32(format!("F(({})>>>0)", compact_i32(value.i32_expr()?))),
            F32ConvertI64S => {
                let (lo, hi) = expect_i64(value)?;
                let lo = compact_i32(&lo);
                let hi = compact_i32(&hi);
                Value::F32(format!("F(+({hi})*4294967296.0+(+(({lo})>>>0)))"))
            }
            F32ConvertI64U => {
                let (lo, hi) = expect_i64(value)?;
                let lo = compact_i32(&lo);
                let hi = compact_i32(&hi);
                Value::F32(format!("F(+(({hi})>>>0)*4294967296.0+(+(({lo})>>>0)))"))
            }
            F32DemoteF64 => Value::F32(format!("F({})", expect_f64(value)?)),
            F64ConvertI32S => Value::F64(format!("+({})", compact_i32(value.i32_expr()?))),
            F64ConvertI32U => Value::F64(format!("+(({})>>>0)", compact_i32(value.i32_expr()?))),
            F64ConvertI64S => {
                let (lo, hi) = expect_i64(value)?;
                let lo = compact_i32(&lo);
                let hi = compact_i32(&hi);
                Value::F64(format!("+(+({hi})*4294967296.0+(+(({lo})>>>0)))"))
            }
            F64ConvertI64U => {
                let (lo, hi) = expect_i64(value)?;
                let lo = compact_i32(&lo);
                let hi = compact_i32(&hi);
                Value::F64(format!("+(+(({hi})>>>0)*4294967296.0+(+(({lo})>>>0)))"))
            }
            F64PromoteF32 => Value::F64(format!("+({})", expect_f32(value)?)),
            I32ReinterpretF32 => Value::I32(format!(
                "rf({})|0",
                compact_float_argument(&expect_f32(value)?)
            )),
            F32ReinterpretI32 => Value::F32(format!("F(+ri({}))", compact_i32(value.i32_expr()?))),
            I64ReinterpretF64 => {
                if let Some((low, high)) = self.f64_bits_for(&value) {
                    Value::I64(low, high)
                } else {
                    let x = compact_float_argument(&expect_f64(value)?);
                    let low = self.temp(ValType::I32);
                    let high = self.temp(ValType::I32);
                    write!(
                        self.body,
                        "{}=rd({})|0;{}=GH()|0;",
                        low.i32_expr()?,
                        x,
                        high.i32_expr()?
                    )
                    .unwrap();
                    Value::I64(low.i32_expr()?.into(), high.i32_expr()?.into())
                }
            }
            F64ReinterpretI64 => {
                let (low, high) = expect_i64(value)?;
                let low = compact_i32(&low);
                let high = compact_i32(&high);
                let expression = format!("+wr({low},{high})");
                self.f64_bits.insert(expression.clone(), (low, high));
                Value::F64(expression)
            }
            I32Extend8S => Value::I32(format!("(({})<<24)>>24", value.i32_expr()?)),
            I32Extend16S => Value::I32(format!("(({})<<16)>>16", value.i32_expr()?)),
            I64Extend8S => {
                let (lo, _) = expect_i64(value)?;
                Value::I64(
                    format!("(({lo})<<24)>>24"),
                    format!("((({lo})<<24)>>24)>>31"),
                )
            }
            I64Extend16S => {
                let (lo, _) = expect_i64(value)?;
                Value::I64(
                    format!("(({lo})<<16)>>16"),
                    format!("((({lo})<<16)>>16)>>31"),
                )
            }
            I64Extend32S => {
                let (lo, _) = expect_i64(value)?;
                Value::I64(lo.clone(), format!("({lo})>>31"))
            }
            I32TruncSatF32S | I32TruncSatF64S => {
                let x = expect_float(value)?;
                self.trunc_i32(x, false, true)?
            }
            I32TruncSatF32U | I32TruncSatF64U => {
                let x = expect_float(value)?;
                self.trunc_i32(x, true, true)?
            }
            I64TruncSatF32S | I64TruncSatF64S => {
                let x = expect_float(value)?;
                self.trunc_i64(x, false, true)?
            }
            I64TruncSatF32U | I64TruncSatF64U => {
                let x = expect_float(value)?;
                self.trunc_i64(x, true, true)?
            }
        };
        self.stack.push(result);
        Ok(())
    }

    fn emit_binary(&mut self, op: BinaryOp) -> Result<(), CompileError> {
        use BinaryOp::*;
        let right = self.pop()?;
        let left = self.pop()?;
        let result = match op {
            I32Eq => cmp_i32(left, right, "==", false)?,
            I32Ne => cmp_i32(left, right, "!=", false)?,
            I32LtS => cmp_i32(left, right, "<", false)?,
            I32LtU => cmp_i32(left, right, "<", true)?,
            I32GtS => cmp_i32(left, right, ">", false)?,
            I32GtU => cmp_i32(left, right, ">", true)?,
            I32LeS => cmp_i32(left, right, "<=", false)?,
            I32LeU => cmp_i32(left, right, "<=", true)?,
            I32GeS => cmp_i32(left, right, ">=", false)?,
            I32GeU => cmp_i32(left, right, ">=", true)?,
            F32Eq | F64Eq => cmp_float(left, right, "==")?,
            F32Ne | F64Ne => cmp_float(left, right, "!=")?,
            F32Lt | F64Lt => cmp_float(left, right, "<")?,
            F32Gt | F64Gt => cmp_float(left, right, ">")?,
            F32Le | F64Le => cmp_float(left, right, "<=")?,
            F32Ge | F64Ge => cmp_float(left, right, ">=")?,
            I32Add => i32_bin(left, right, "+")?,
            I32Sub => i32_bin(left, right, "-")?,
            I32Mul => {
                let (a, b) = expect_i32_pair(left, right)?;
                Value::I32(format!("U({a},{b})"))
            }
            I32DivS | I32DivU | I32RemS | I32RemU => self.i32_divrem(left, right, op)?,
            I32And => i32_bin(left, right, "&")?,
            I32Or => i32_bin(left, right, "|")?,
            I32Xor => i32_bin(left, right, "^")?,
            I32Shl => i32_bin(left, right, "<<")?,
            I32ShrS => i32_bin(left, right, ">>")?,
            I32ShrU => {
                let (a, b) = expect_i32_pair(left, right)?;
                let a = compact_unsigned_i32_operand(&a);
                let b = compact_operand(&b);
                Value::I32(format!("({a}>>>{b})|0"))
            }
            I32Rotl => {
                let (a, b) = expect_i32_pair(left, right)?;
                let a = compact_operand(&a);
                let b = compact_operand(&b);
                Value::I32(format!("({a}<<{b})|({a}>>>(-{b}))"))
            }
            I32Rotr => {
                let (a, b) = expect_i32_pair(left, right)?;
                let a = compact_operand(&a);
                let b = compact_operand(&b);
                Value::I32(format!("({a}>>>{b})|({a}<<(-{b}))"))
            }
            I64Eq | I64Ne | I64LtS | I64LtU | I64GtS | I64GtU | I64LeS | I64LeU | I64GeS
            | I64GeU => i64_compare(left, right, op)?,
            I64Add | I64Sub | I64Mul | I64DivS | I64DivU | I64RemS | I64RemU | I64And | I64Or
            | I64Xor | I64Shl | I64ShrS | I64ShrU | I64Rotl | I64Rotr => {
                self.i64_binary(left, right, op)?
            }
            F32Add => float_bin(left, right, "+", true)?,
            F32Sub => float_bin(left, right, "-", true)?,
            F32Mul => float_bin(left, right, "*", true)?,
            F32Div => float_bin(left, right, "/", true)?,
            F64Add => float_bin(left, right, "+", false)?,
            F64Sub => float_bin(left, right, "-", false)?,
            F64Mul => float_bin(left, right, "*", false)?,
            F64Div => float_bin(left, right, "/", false)?,
            F32Min => float_helper(left, right, "mn", true)?,
            F32Max => float_helper(left, right, "mx", true)?,
            F32Copysign => float_helper(left, right, "cs", true)?,
            F64Min => float_helper(left, right, "mn", false)?,
            F64Max => float_helper(left, right, "mx", false)?,
            F64Copysign => float_helper(left, right, "cs", false)?,
        };
        self.stack.push(result);
        Ok(())
    }

    fn i32_divrem(
        &mut self,
        left: Value,
        right: Value,
        op: BinaryOp,
    ) -> Result<Value, CompileError> {
        let (a, b) = expect_i32_pair(left, right)?;
        let a_i32 = compact_i32(&a);
        let b_i32 = compact_i32(&b);
        let b_literal = i32_literal(&b);
        if self.module.options.preserve_traps {
            if b_literal.is_none_or(|value| value == 0)
                && !nonzero_guard_dominates(&self.body, &b_i32)
            {
                write!(self.body, "if(({b_i32})==0)X();").unwrap();
            }
            if matches!(op, BinaryOp::I32DivS) {
                match b_literal {
                    Some(-1) => write!(self.body, "if(({a_i32})==(-2147483648|0))X();").unwrap(),
                    None => write!(
                        self.body,
                        "if(({a_i32})==(-2147483648|0)){{if(({b_i32})==(-1|0))X();}}"
                    )
                    .unwrap(),
                    _ => {}
                }
            }
        }
        let unsigned_divisor = b_literal.map(|value| value as u32);
        let expr = match op {
            BinaryOp::I32DivS => format!("({a_i32})/({b_i32})|0"),
            BinaryOp::I32DivU if unsigned_divisor == Some(255) && is_unsigned_u16(&a) => {
                format!("U({},32897)>>>23", compact_i32(&a))
            }
            BinaryOp::I32DivU if unsigned_divisor.is_some_and(u32::is_power_of_two) => format!(
                "{}>>>{}",
                compact_unsigned_i32_operand(&a),
                unsigned_divisor.unwrap().trailing_zeros()
            ),
            BinaryOp::I32DivU => format!(
                "({}>>>0)/({}>>>0)>>>0|0",
                compact_unsigned_i32_operand(&a),
                compact_unsigned_i32_operand(&b)
            ),
            BinaryOp::I32RemS => format!("({a_i32})%({b_i32})|0"),
            BinaryOp::I32RemU if unsigned_divisor.is_some_and(u32::is_power_of_two) => {
                format!("({})&{}", compact_i32(&a), unsigned_divisor.unwrap() - 1)
            }
            BinaryOp::I32RemU => format!(
                "({}>>>0)%({}>>>0)>>>0|0",
                compact_unsigned_i32_operand(&a),
                compact_unsigned_i32_operand(&b)
            ),
            _ => unreachable!(),
        };
        Ok(Value::I32(expr))
    }

    fn i64_binary(
        &mut self,
        left: Value,
        right: Value,
        op: BinaryOp,
    ) -> Result<Value, CompileError> {
        let (al, ah) = expect_i64(left)?;
        let (bl, bh) = expect_i64(right)?;
        let al_low_zero = is_i32_zero(&al);
        let bl_low_zero = is_i32_zero(&bl);
        let add_carry = if reusable_i32_expression(&al) || !reusable_i32_expression(&bl) {
            al.clone()
        } else {
            bl.clone()
        };
        let al = compact_i32(&al);
        let ah = compact_i32(&ah);
        let bl = compact_i32(&bl);
        let bh = compact_i32(&bh);
        let add_carry = compact_i32(&add_carry);
        let ah_zero = is_i32_zero(&ah);
        let bh_zero = is_i32_zero(&bh);
        let low = self.temp(ValType::I32);
        let low_name = low.i32_expr()?.to_string();
        let lazy_high = match op {
            BinaryOp::I64Add => {
                if al_low_zero {
                    write!(self.body, "{low_name}={bl}|0;").unwrap();
                } else if bl_low_zero {
                    write!(self.body, "{low_name}={al}|0;").unwrap();
                } else {
                    write!(self.body, "{low_name}=({al})+({bl})|0;").unwrap();
                }
                if al_low_zero || bl_low_zero {
                    match (ah_zero, bh_zero) {
                        (true, true) => "0".into(),
                        (true, false) => bh,
                        (false, true) => ah,
                        (false, false) => format!("({ah})+({bh})|0"),
                    }
                } else {
                    match (ah_zero, bh_zero) {
                        (true, true) => format!("(({low_name}>>>0)<(({add_carry})>>>0))|0"),
                        (true, false) => {
                            format!("({bh})+(({low_name}>>>0)<(({add_carry})>>>0))|0")
                        }
                        (false, true) => {
                            format!("({ah})+(({low_name}>>>0)<(({add_carry})>>>0))|0")
                        }
                        (false, false) => {
                            format!("({ah})+({bh})+(({low_name}>>>0)<(({add_carry})>>>0))|0")
                        }
                    }
                }
            }
            BinaryOp::I64Sub => {
                if bl_low_zero {
                    write!(self.body, "{low_name}={al}|0;").unwrap();
                    match (ah_zero, bh_zero) {
                        (true, true) => "0".into(),
                        (true, false) => format!("(0|0)-({bh})|0"),
                        (false, true) => ah,
                        (false, false) => format!("({ah})-({bh})|0"),
                    }
                } else {
                    write!(self.body, "{low_name}=({al})-({bl})|0;").unwrap();
                    match (ah_zero, bh_zero) {
                        (true, true) => format!("(0|0)-(((({al})>>>0)<(({bl})>>>0))|0)|0"),
                        (true, false) => {
                            format!("((0|0)-({bh})|0)-(((({al})>>>0)<(({bl})>>>0))|0)|0")
                        }
                        (false, true) => {
                            format!("({ah})-(((({al})>>>0)<(({bl})>>>0))|0)|0")
                        }
                        (false, false) => {
                            format!("({ah})-({bh})-(((({al})>>>0)<(({bl})>>>0))|0)|0")
                        }
                    }
                }
            }
            BinaryOp::I64Mul => {
                write!(self.body, "{low_name}=U({al},{bl})|0;").unwrap();
                format!("$m({al},{ah},{bl},{bh})|0")
            }
            BinaryOp::I64And => {
                write!(self.body, "{low_name}=({al})&({bl});").unwrap();
                if ah_zero || bh_zero {
                    "0".into()
                } else {
                    format!("({ah})&({bh})")
                }
            }
            BinaryOp::I64Or | BinaryOp::I64Xor => {
                let operator = if matches!(op, BinaryOp::I64Or) {
                    '|'
                } else {
                    '^'
                };
                write!(self.body, "{low_name}=({al}){operator}({bl});").unwrap();
                match (ah_zero, bh_zero) {
                    (true, true) => "0".into(),
                    (true, false) => bh,
                    (false, true) => ah,
                    (false, false) => format!("({ah}){operator}({bh})"),
                }
            }
            _ => {
                let code = match op {
                    BinaryOp::I64Shl => 6,
                    BinaryOp::I64ShrS => 7,
                    BinaryOp::I64ShrU => 8,
                    BinaryOp::I64Rotl => 9,
                    BinaryOp::I64Rotr => 10,
                    BinaryOp::I64DivS => 11,
                    BinaryOp::I64DivU => 12,
                    BinaryOp::I64RemS => 13,
                    BinaryOp::I64RemU => 14,
                    _ => unreachable!(),
                };
                let high = self.temp(ValType::I32);
                let high_name = high.i32_expr()?.to_string();
                if let Some(helper) = ["$g", "$h", "$i"].get((code - 6) as usize) {
                    write!(
                        self.body,
                        "{low_name}={helper}({al},{ah},{bl},{bh})|0;{high_name}=$ih|0;"
                    )
                    .unwrap();
                } else {
                    write!(
                        self.body,
                        "{low_name}=W({code}|0,{al},{ah},{bl},{bh})|0;{high_name}=GH()|0;"
                    )
                    .unwrap();
                }
                return Ok(Value::I64(low_name, high_name));
            }
        };
        Ok(Value::I64(low_name, lazy_high))
    }

    fn trunc_i32(
        &mut self,
        value: String,
        unsigned: bool,
        saturating: bool,
    ) -> Result<Value, CompileError> {
        let temp = self.temp(ValType::I32);
        if !self.module.options.preserve_traps && !saturating {
            write!(self.body, "{}=tr(+({value}))|0;", temp.i32_expr()?).unwrap();
        } else {
            write!(
                self.body,
                "{}=Y(+({value}),{}|0,{}|0)|0;",
                temp.i32_expr()?,
                unsigned as u8,
                saturating as u8
            )
            .unwrap();
        }
        Ok(temp)
    }

    fn trunc_i64(
        &mut self,
        value: String,
        unsigned: bool,
        saturating: bool,
    ) -> Result<Value, CompileError> {
        let low = self.temp(ValType::I32);
        let high = self.temp(ValType::I32);
        write!(
            self.body,
            "{}=y(+({value}),{}|0,{}|0)|0;{}=GH()|0;",
            low.i32_expr()?,
            unsigned as u8,
            saturating as u8,
            high.i32_expr()?
        )
        .unwrap();
        Ok(Value::I64(low.i32_expr()?.into(), high.i32_expr()?.into()))
    }

    fn emit_load(&mut self, op: LoadOp, arg: MemArg) -> Result<(), CompileError> {
        let memory = self
            .module
            .module
            .memories
            .get(arg.memory as usize)
            .ok_or_else(|| self.internal("invalid memory index"))?;
        let compact = arg.memory == 0
            && self.module.module.memories.len() == 1
            && !memory.memory64
            && arg.offset <= u32::MAX as u64;
        let (lo, hi) = self.pop_address(memory.memory64)?;
        let code = load_code(op);
        let compact_address = compact.then(|| compact_memory_address(&lo, arg.offset));
        let lo_i32 = compact_i32(&lo);
        let hi_i32 = compact_i32(&hi);
        let offset_helper = self.module.load_offset_helpers.get(&(arg.offset, code));
        let direct = self.module.direct_memory_size.is_some() && compact;
        let call = if let Some(address) = &compact_address {
            if arg.offset == 0 {
                if direct {
                    format!("$L{code}({address})")
                } else {
                    format!("l({address},{code})")
                }
            } else if let Some(helper) = offset_helper {
                format!("{helper}({address})")
            } else if direct {
                format!("$L{code}(B0({address},{})|0)", arg.offset)
            } else {
                format!("l2({address},{},{code})", arg.offset)
            }
        } else {
            format!(
                "LI({}|0,{lo_i32},{hi_i32},+{},{}|0)",
                arg.memory, arg.offset, code
            )
        };
        let float_call = compact_address.as_ref().map(|address| {
            if arg.offset == 0 {
                if direct {
                    format!("$F{code}({address})")
                } else {
                    format!("lf({address},{code})")
                }
            } else if let Some(helper) = offset_helper {
                format!("{helper}({address})")
            } else if direct {
                format!("$F{code}(B0({address},{})|0)", arg.offset)
            } else {
                format!("lf2({address},{},{code})", arg.offset)
            }
        });
        match op {
            LoadOp::I64 => {
                let low = self.temp(ValType::I32);
                let high = self.temp(ValType::I32);
                if direct {
                    write!(
                        self.body,
                        "{}={call}|0;{}=$ih|0;",
                        low.i32_expr()?,
                        high.i32_expr()?
                    )
                    .unwrap();
                } else {
                    write!(
                        self.body,
                        "{}={call}|0;{}=GH()|0;",
                        low.i32_expr()?,
                        high.i32_expr()?
                    )
                    .unwrap();
                }
                self.stack
                    .push(Value::I64(low.i32_expr()?.into(), high.i32_expr()?.into()));
            }
            LoadOp::I64_8S
            | LoadOp::I64_8U
            | LoadOp::I64_16S
            | LoadOp::I64_16U
            | LoadOp::I64_32S
            | LoadOp::I64_32U => {
                let low = self.temp(ValType::I32);
                write!(self.body, "{}={call}|0;", low.i32_expr()?).unwrap();
                let low_name = low.i32_expr()?.to_string();
                let high = if matches!(op, LoadOp::I64_8S | LoadOp::I64_16S | LoadOp::I64_32S) {
                    format!("({low_name})>>31")
                } else {
                    "0".into()
                };
                self.stack.push(Value::I64(low_name, high));
            }
            LoadOp::F32 => {
                let value = self.temp(ValType::F32);
                if let Some(call) = &float_call {
                    write!(self.body, "{}=F(+{call});", value.components()[0]).unwrap();
                } else {
                    write!(
                        self.body,
                        "{}=F(+LF({}|0,{lo_i32},{hi_i32},+{},{}|0));",
                        value.components()[0],
                        arg.memory,
                        arg.offset,
                        code
                    )
                    .unwrap();
                }
                self.stack.push(value);
            }
            LoadOp::F64 => {
                let value = self.temp(ValType::F64);
                let value_name = value.components()[0].to_string();
                let (raw_low, raw_high) = self.ensure_f64_bits(&value_name);
                if let Some(call) = &float_call {
                    write!(self.body, "{value_name}=+{call};").unwrap();
                } else {
                    write!(
                        self.body,
                        "{value_name}=+LF({}|0,{lo_i32},{hi_i32},+{},{}|0);",
                        arg.memory, arg.offset, code
                    )
                    .unwrap();
                }
                if direct {
                    write!(self.body, "{raw_low}=$rl|0;{raw_high}=$rh|0;").unwrap();
                } else {
                    let raw_call = if let Some(address) = &compact_address {
                        if arg.offset == 0 {
                            format!("l({address},1)")
                        } else {
                            format!("l2({address},{},1)", arg.offset)
                        }
                    } else {
                        format!("LI({}|0,{lo_i32},{hi_i32},+{},1|0)", arg.memory, arg.offset)
                    };
                    write!(self.body, "{raw_low}={raw_call}|0;{raw_high}=GH()|0;").unwrap();
                }
                self.stack.push(value);
            }
            _ => {
                let value = self.temp(ValType::I32);
                write!(self.body, "{}={call}|0;", value.i32_expr()?).unwrap();
                self.stack.push(value);
            }
        }
        Ok(())
    }

    fn emit_store(&mut self, op: StoreOp, arg: MemArg) -> Result<(), CompileError> {
        let value = self.pop()?;
        let f64_bits = if matches!(op, StoreOp::F64) {
            self.f64_bits_for(&value)
        } else {
            None
        };
        let memory = self
            .module
            .module
            .memories
            .get(arg.memory as usize)
            .ok_or_else(|| self.internal("invalid memory index"))?;
        let compact = arg.memory == 0
            && self.module.module.memories.len() == 1
            && !memory.memory64
            && arg.offset <= u32::MAX as u64;
        let direct = self.module.direct_memory_size.is_some() && compact;
        let (lo, hi) = self.pop_address(memory.memory64)?;
        let lo_i32 = compact_i32(&lo);
        let hi_i32 = compact_i32(&hi);
        let code = store_code(op);
        let offset_helper = self.module.store_offset_helpers.get(&(arg.offset, code));
        if let Some((value_low, value_high)) = f64_bits {
            let value_low = compact_i32(&value_low);
            let value_high = compact_i32(&value_high);
            if let Some(address) = compact.then(|| compact_memory_address(&lo, arg.offset)) {
                if direct {
                    let address = if arg.offset == 0 {
                        address
                    } else {
                        format!("B0({address},{})|0", arg.offset)
                    };
                    write!(self.body, "$S1({address},{value_low},{value_high});").unwrap();
                } else if arg.offset == 0 {
                    write!(self.body, "st({address},1,{value_low},{value_high});").unwrap();
                } else {
                    write!(
                        self.body,
                        "st2({address},{},1,{value_low},{value_high});",
                        arg.offset
                    )
                    .unwrap();
                }
            } else {
                write!(
                    self.body,
                    "SI({}|0,{lo_i32},{hi_i32},+{},1|0,{value_low},{value_high});",
                    arg.memory, arg.offset
                )
                .unwrap();
            }
            return Ok(());
        }
        if let Some(address) = compact.then(|| compact_memory_address(&lo, arg.offset)) {
            match op {
                StoreOp::F32 | StoreOp::F64 => {
                    let value = coerce(ValType::F64, &expect_float(value)?);
                    if arg.offset == 0 {
                        if direct {
                            write!(self.body, "$D{code}({address},{value});").unwrap();
                        } else {
                            write!(self.body, "sf({address},{code},{value});").unwrap();
                        }
                    } else if let Some(helper) = offset_helper {
                        write!(self.body, "{helper}({address},{value});").unwrap();
                    } else if direct {
                        write!(
                            self.body,
                            "$D{code}(B0({address},{})|0,{value});",
                            arg.offset
                        )
                        .unwrap();
                    } else {
                        write!(self.body, "sf2({address},{},{code},{value});", arg.offset).unwrap();
                    }
                }
                StoreOp::I64 | StoreOp::I64_8 | StoreOp::I64_16 | StoreOp::I64_32 => {
                    let (value_lo, value_hi) = expect_i64(value)?;
                    let value_lo = compact_i32(&value_lo);
                    let value_hi = compact_i32(&value_hi);
                    if arg.offset == 0 {
                        if direct {
                            if code == 1 {
                                write!(self.body, "$S1({address},{value_lo},{value_hi});").unwrap();
                            } else {
                                write!(self.body, "$S{code}({address},{value_lo});").unwrap();
                            }
                        } else {
                            write!(self.body, "st({address},{code},{value_lo},{value_hi});")
                                .unwrap();
                        }
                    } else if let Some(helper) = offset_helper {
                        if code == 1 {
                            write!(self.body, "{helper}({address},{value_lo},{value_hi});")
                                .unwrap();
                        } else {
                            write!(self.body, "{helper}({address},{value_lo});").unwrap();
                        }
                    } else if direct {
                        if code == 1 {
                            write!(
                                self.body,
                                "$S1(B0({address},{})|0,{value_lo},{value_hi});",
                                arg.offset
                            )
                            .unwrap();
                        } else {
                            write!(
                                self.body,
                                "$S{code}(B0({address},{})|0,{value_lo});",
                                arg.offset
                            )
                            .unwrap();
                        }
                    } else {
                        write!(
                            self.body,
                            "st2({address},{},{code},{value_lo},{value_hi});",
                            arg.offset
                        )
                        .unwrap();
                    }
                }
                _ => {
                    let value = compact_i32(value.i32_expr()?);
                    if arg.offset == 0 {
                        if direct {
                            write!(self.body, "$S{code}({address},{value});").unwrap();
                        } else {
                            write!(self.body, "st({address},{code},{value},0);").unwrap();
                        }
                    } else if let Some(helper) = offset_helper {
                        write!(self.body, "{helper}({address},{value});").unwrap();
                    } else if direct {
                        write!(
                            self.body,
                            "$S{code}(B0({address},{})|0,{value});",
                            arg.offset
                        )
                        .unwrap();
                    } else {
                        write!(self.body, "st2({address},{},{code},{value},0);", arg.offset)
                            .unwrap();
                    }
                }
            }
            return Ok(());
        }
        match op {
            StoreOp::F32 | StoreOp::F64 => {
                let value = expect_float(value)?;
                let value = coerce(ValType::F64, &value);
                write!(
                    self.body,
                    "SF({}|0,{lo_i32},{hi_i32},+{},{}|0,{value});",
                    arg.memory, arg.offset, code
                )
                .unwrap();
            }
            StoreOp::I64 | StoreOp::I64_8 | StoreOp::I64_16 | StoreOp::I64_32 => {
                let (value_lo, value_hi) = expect_i64(value)?;
                write!(
                    self.body,
                    "SI({}|0,{lo_i32},{hi_i32},+{},{}|0,{},{});",
                    arg.memory,
                    arg.offset,
                    code,
                    compact_i32(&value_lo),
                    compact_i32(&value_hi)
                )
                .unwrap();
            }
            _ => {
                let value = compact_i32(value.i32_expr()?);
                write!(
                    self.body,
                    "SI({}|0,{lo_i32},{hi_i32},+{},{}|0,{value},0);",
                    arg.memory, arg.offset, code
                )
                .unwrap();
            }
        }
        Ok(())
    }

    fn emit_memory_copy(&mut self, dst: u32, src: u32) -> Result<(), CompileError> {
        let count = compact_i32(&self.pop_i32()?.to_string());
        let source = compact_i32(&self.pop_i32()?.to_string());
        let dest = compact_i32(&self.pop_i32()?.to_string());
        write!(self.body, "AB({dst}|0,{src}|0,{dest},{source},{count});").unwrap();
        Ok(())
    }
    fn emit_memory_fill(&mut self, memory: u32) -> Result<(), CompileError> {
        let count = compact_i32(&self.pop_i32()?.to_string());
        let value = compact_i32(&self.pop_i32()?.to_string());
        let dest = compact_i32(&self.pop_i32()?.to_string());
        write!(self.body, "AC({memory}|0,{dest},{value},{count});").unwrap();
        Ok(())
    }
    fn emit_memory_init(&mut self, data: u32, memory: u32) -> Result<(), CompileError> {
        let count = compact_i32(&self.pop_i32()?.to_string());
        let source = compact_i32(&self.pop_i32()?.to_string());
        let dest = compact_i32(&self.pop_i32()?.to_string());
        write!(self.body, "K({data}|0,{memory}|0,{dest},{source},{count});").unwrap();
        Ok(())
    }

    fn emit_simd(&mut self, op: &SimdOp) -> Result<(), CompileError> {
        use SimdOp::*;
        let value = match op {
            Const(bytes) => Value::V128(std::array::from_fn(|i| {
                i32::from_le_bytes(bytes[i * 4..i * 4 + 4].try_into().unwrap()).to_string()
            })),
            I32x4Splat => {
                let x = self.pop_i32()?.to_string();
                Value::V128(std::array::from_fn(|_| x.clone()))
            }
            F32x4Splat => {
                let x = expect_f32(self.pop()?)?;
                Value::V128(std::array::from_fn(|_| format!("rf(+({x}))|0")))
            }
            I32x4Add | I32x4Sub | I32x4Mul => {
                let b = expect_v128(self.pop()?)?;
                let a = expect_v128(self.pop()?)?;
                Value::V128(std::array::from_fn(|i| match op {
                    I32x4Add => format!("({}+{})|0", a[i], b[i]),
                    I32x4Sub => format!("({}-{})|0", a[i], b[i]),
                    _ => format!("U({},{})|0", a[i], b[i]),
                }))
            }
            F32x4Add | F32x4Sub | F32x4Mul | F32x4Div => {
                let b = expect_v128(self.pop()?)?;
                let a = expect_v128(self.pop()?)?;
                let symbol = match op {
                    F32x4Add => "+",
                    F32x4Sub => "-",
                    F32x4Mul => "*",
                    _ => "/",
                };
                Value::V128(std::array::from_fn(|i| {
                    format!("rf(+(F((+ri(({})|0)){symbol}(+ri(({})|0)))))|0", a[i], b[i])
                }))
            }
            F32x4Abs | F32x4Neg | F32x4Sqrt => {
                let a = expect_v128(self.pop()?)?;
                Value::V128(std::array::from_fn(|i| match op {
                    F32x4Abs => format!("rf(+(F(Ma(+ri(({})|0)))))|0", a[i]),
                    F32x4Neg => format!("rf(+(F(-(+ri(({})|0)))))|0", a[i]),
                    _ => format!("rf(+(F(Ms(+ri(({})|0)))))|0", a[i]),
                }))
            }
            I32x4Shl => {
                let n = self.pop_i32()?.to_string();
                let a = expect_v128(self.pop()?)?;
                Value::V128(std::array::from_fn(|i| format!("({}<<({n}&31))|0", a[i])))
            }
            I32x4Extract(lane) => {
                let a = expect_v128(self.pop()?)?;
                Value::I32(a[*lane as usize].clone())
            }
            I32x4Replace(lane) => {
                let x = self.pop_i32()?.to_string();
                let mut a = expect_v128(self.pop()?)?;
                a[*lane as usize] = x;
                Value::V128(a)
            }
            F32x4Extract(lane) => {
                let a = expect_v128(self.pop()?)?;
                Value::F32(format!("F(+ri(({})|0))", a[*lane as usize]))
            }
            F32x4Replace(lane) => {
                let x = expect_f32(self.pop()?)?;
                let mut a = expect_v128(self.pop()?)?;
                a[*lane as usize] = format!("rf(+({x}))|0");
                Value::V128(a)
            }
            I8x16Shuffle(lanes) => {
                let b = expect_v128(self.pop()?)?;
                let a = expect_v128(self.pop()?)?;
                Value::V128(std::array::from_fn(|word| {
                    let mut parts = Vec::with_capacity(4);
                    for byte in 0..4 {
                        let lane = lanes[word * 4 + byte] as usize;
                        let source = if lane < 16 {
                            &a[lane / 4]
                        } else {
                            &b[(lane - 16) / 4]
                        };
                        let source_shift = (lane % 4) * 8;
                        let target_shift = byte * 8;
                        parts.push(if target_shift == 0 {
                            format!("(({source}>>>{source_shift})&255)")
                        } else {
                            format!("((({source}>>>{source_shift})&255)<<{target_shift})")
                        });
                    }
                    format!("({})|0", parts.join("|"))
                }))
            }
            V128And | V128Or | V128Xor => {
                let b = expect_v128(self.pop()?)?;
                let a = expect_v128(self.pop()?)?;
                let symbol = match op {
                    V128And => "&",
                    V128Or => "|",
                    _ => "^",
                };
                Value::V128(std::array::from_fn(|i| {
                    format!("({}{symbol}{})|0", a[i], b[i])
                }))
            }
            V128Not => {
                let a = expect_v128(self.pop()?)?;
                Value::V128(std::array::from_fn(|i| format!("~{}", a[i])))
            }
            V128Bitselect => {
                let c = expect_v128(self.pop()?)?;
                let b = expect_v128(self.pop()?)?;
                let a = expect_v128(self.pop()?)?;
                Value::V128(std::array::from_fn(|i| {
                    format!("(({}&{})|({}&~{}))|0", a[i], c[i], b[i], c[i])
                }))
            }
            V128AnyTrue => {
                let a = expect_v128(self.pop()?)?;
                Value::I32(format!("(({}|{}|{}|{})!=0)|0", a[0], a[1], a[2], a[3]))
            }
            V128Load(arg) => {
                let memory = &self.module.module.memories[arg.memory as usize];
                let (lo, hi) = self.pop_address(memory.memory64)?;
                Value::V128(std::array::from_fn(|i| {
                    format!(
                        "VL({}|0,({lo})|0,({hi})|0,+{},{}|0)|0",
                        arg.memory, arg.offset, i
                    )
                }))
            }
            V128Store(arg) => {
                let a = expect_v128(self.pop()?)?;
                let memory = &self.module.module.memories[arg.memory as usize];
                let (lo, hi) = self.pop_address(memory.memory64)?;
                write!(
                    self.body,
                    "VS({}|0,({lo})|0,({hi})|0,+{},({})|0,({})|0,({})|0,({})|0);",
                    arg.memory, arg.offset, a[0], a[1], a[2], a[3]
                )
                .unwrap();
                return Ok(());
            }
            Other(name) => {
                return Err(CompileError::unsupported(
                    "simd",
                    None,
                    format!("SIMD instruction {name}"),
                ));
            }
        };
        self.stack.push(value);
        Ok(())
    }

    fn pop_address(&mut self, memory64: bool) -> Result<(String, String), CompileError> {
        if memory64 {
            let (lo, hi) = expect_i64(self.pop()?)?;
            Ok((lo, hi))
        } else {
            Ok((self.pop_i32()?.to_string(), "0".into()))
        }
    }
    fn pop_table_index(&mut self, table: u32) -> Result<String, CompileError> {
        if self.module.module.tables[table as usize].table64 {
            let (lo, hi) = expect_i64(self.pop()?)?;
            write!(self.body, "if(({hi})|0)X();").unwrap();
            Ok(lo)
        } else {
            self.pop_i32()
        }
    }
    fn ensure_table_zero(&self, table: u32) -> Result<(), CompileError> {
        if table == 0 {
            Ok(())
        } else {
            Err(CompileError::unsupported(
                "multiple tables",
                None,
                "only table zero is supported",
            ))
        }
    }
    fn pop(&mut self) -> Result<Value, CompileError> {
        let value = self
            .stack
            .pop()
            .ok_or_else(|| self.internal("operand stack underflow"))?;
        for (index, (base, ty)) in self.temp_slots.iter().enumerate() {
            if self.instruction_temps.contains(&index) {
                continue;
            }
            let candidate = named_value(*ty, base);
            if candidate
                .components()
                .into_iter()
                .any(|name| value.mentions_identifier(name))
            {
                self.instruction_temps.push(index);
            }
        }
        Ok(value)
    }
    fn pop_i32(&mut self) -> Result<String, CompileError> {
        let value = self.pop()?;
        match value {
            Value::I32(v) | Value::Ref(v) => Ok(v),
            _ => Err(self.internal("expected i32")),
        }
    }
    fn pop_types(&mut self, types: &[ValType]) -> Result<Vec<Value>, CompileError> {
        let mut values = Vec::with_capacity(types.len());
        for &ty in types.iter().rev() {
            let value = self.pop()?;
            if !type_compatible(value.ty(), ty) {
                return Err(self.internal("operand type mismatch"));
            }
            values.push(value);
        }
        values.reverse();
        Ok(values)
    }
    fn peek_types(&self, types: &[ValType]) -> Result<Vec<Value>, CompileError> {
        if self.stack.len() < types.len() {
            return Err(self.internal("operand stack underflow"));
        }
        let values = self.stack[self.stack.len() - types.len()..].to_vec();
        for (value, &ty) in values.iter().zip(types) {
            if !type_compatible(value.ty(), ty) {
                return Err(self.internal("operand type mismatch"));
            }
        }
        Ok(values)
    }
    fn local(&self, index: u32) -> Result<&Value, CompileError> {
        self.locals
            .get(index as usize)
            .ok_or_else(|| self.internal("invalid local index"))
    }
    fn branch_types(&self, index: usize) -> Vec<ValType> {
        let c = &self.controls[index];
        if c.kind == ControlKind::Loop {
            c.params.iter().map(Value::ty).collect()
        } else {
            c.results.clone()
        }
    }
    fn target_index(&self, depth: u32) -> Result<usize, CompileError> {
        self.controls
            .len()
            .checked_sub(1 + depth as usize)
            .ok_or_else(|| self.internal("invalid branch depth"))
    }
    fn temp(&mut self, ty: ValType) -> Value {
        // Values from earlier instructions are live only through the operand/control state.
        let reusable = self
            .temp_slots
            .iter()
            .enumerate()
            .find(|(index, (base, slot_ty))| {
                *slot_ty == ty
                    && !self.instruction_temps.contains(index)
                    && !self.temp_is_live(base, ty)
            })
            .map(|(index, _)| index);
        let index = if let Some(index) = reusable {
            index
        } else {
            let base = local_ident(self.temp_index);
            self.temp_index += 1;
            let value = named_value(ty, &base);
            for name in value.components() {
                self.declarations
                    .push((name.to_string(), component_decl_type(ty, name)));
            }
            self.temp_slots.push((base, ty));
            self.temp_slots.len() - 1
        };
        self.instruction_temps.push(index);
        named_value(ty, &self.temp_slots[index].0)
    }

    fn temp_is_live(&self, base: &str, ty: ValType) -> bool {
        let candidate = named_value(ty, base);
        let uses_candidate = |value: &Value| {
            candidate
                .components()
                .into_iter()
                .any(|name| value.mentions_identifier(name))
        };
        self.stack.iter().any(uses_candidate)
            || self
                .controls
                .iter()
                .any(|control| control.params.iter().any(uses_candidate))
            || self
                .controls
                .iter()
                .any(|control| control.preserved_stack.iter().any(uses_candidate))
            || self
                .controls
                .iter()
                .any(|control| control.result_values.iter().any(uses_candidate))
    }
    fn f64_bits_for(&self, value: &Value) -> Option<(String, String)> {
        let Value::F64(expression) = value else {
            return None;
        };
        self.f64_bits.get(expression).cloned()
    }

    fn ensure_f64_bits(&mut self, expression: &str) -> (String, String) {
        if let Some(bits) = self.f64_bits.get(expression) {
            return bits.clone();
        }
        let low = local_ident(self.temp_index);
        self.temp_index += 1;
        let high = local_ident(self.temp_index);
        self.temp_index += 1;
        self.declarations.push((low.clone(), ValType::I32));
        self.declarations.push((high.clone(), ValType::I32));
        self.f64_bits
            .insert(expression.to_string(), (low.clone(), high.clone()));
        (low, high)
    }

    fn retarget_last_temp_assignment(&mut self, target: &Value, value: &Value) -> bool {
        if matches!(target, Value::F64(_)) || matches!(value, Value::F64(_)) {
            return false;
        }
        let value_components = value.components();
        let target_components = target.components();
        let (Some(source), Some(target)) = (value_components.first(), target_components.first())
        else {
            return false;
        };
        if value_components.len() != target_components.len()
            || value_components
                .iter()
                .skip(1)
                .any(|component| expression_mentions_identifier(component, source))
            || !self.temp_slots.iter().any(|(base, ty)| {
                named_value(*ty, base)
                    .components()
                    .iter()
                    .any(|component| component == source)
            })
            || !self.body.ends_with(';')
        {
            return false;
        }
        let statement_end = self.body.len() - 1;
        let statement_start = self.body[..statement_end]
            .rfind([';', '{', '}'])
            .map_or(0, |index| index + 1);
        if !self.body[statement_start..statement_end].starts_with(&format!("{source}=")) {
            return false;
        }
        self.body
            .replace_range(statement_start..statement_start + source.len(), target);
        for (target, source) in target_components.iter().zip(&value_components).skip(1) {
            write!(
                self.body,
                "{target}={};",
                coerce_assignment(value_component_type(value, source), source)
            )
            .unwrap();
        }
        true
    }

    fn inline_last_temp_return(&mut self, values: &[Value]) -> bool {
        let [value] = values else {
            return false;
        };
        let value_components = value.components();
        let [source] = value_components.as_slice() else {
            return false;
        };
        if !self.temp_slots.iter().any(|(base, ty)| {
            named_value(*ty, base)
                .components()
                .iter()
                .any(|component| component == source)
        }) || !self.body.ends_with(';')
        {
            return false;
        }
        let statement_end = self.body.len() - 1;
        let statement_start = self.body[..statement_end]
            .rfind([';', '{', '}'])
            .map_or(0, |index| index + 1);
        let expression_start = statement_start + source.len() + 1;
        if self.body.as_bytes().get(statement_start..expression_start)
            != Some(format!("{source}=").as_bytes())
        {
            return false;
        }
        let expression = self.body[expression_start..statement_end].to_string();
        self.body.truncate(statement_start);
        write!(self.body, "return {expression};").unwrap();
        true
    }

    fn assign_values(&mut self, targets: &[Value], values: &[Value]) {
        let retargeted = if let ([target], [value]) = (targets, values) {
            self.retarget_last_temp_assignment(target, value)
        } else {
            false
        };
        if !retargeted {
            for (target, value) in targets.iter().zip(values) {
                self.assign(target, value);
            }
        }
    }

    fn assign(&mut self, target: &Value, value: &Value) {
        let source_bits = self.f64_bits_for(value);
        for (a, b) in target.components().iter().zip(value.components()) {
            write!(
                self.body,
                "{a}={};",
                coerce_assignment(value_component_type(value, b), b)
            )
            .unwrap();
        }
        if let Value::F64(target_expression) = target {
            let (target_low, target_high) = self.ensure_f64_bits(target_expression);
            if let Some((source_low, source_high)) = source_bits {
                write!(
                    self.body,
                    "{target_low}={};{target_high}={};",
                    coerce_assignment(ValType::I32, &source_low),
                    coerce_assignment(ValType::I32, &source_high)
                )
                .unwrap();
            } else {
                write!(
                    self.body,
                    "{target_low}=rd(+({target_expression}))|0;{target_high}=GH()|0;"
                )
                .unwrap();
            }
        }
    }
    fn preserve_assignment_source(&mut self, target: &Value, value: Value) -> Value {
        if target
            .components()
            .into_iter()
            .any(|name| value.mentions_identifier(name))
        {
            self.materialize(&value)
        } else {
            value
        }
    }

    fn materialize(&mut self, value: &Value) -> Value {
        let snapshot = self.temp(value.ty());
        self.assign(&snapshot, value);
        snapshot
    }
    fn preserve_local_values(&mut self, target: &Value) {
        let mut names = target
            .components()
            .into_iter()
            .map(str::to_string)
            .collect::<Vec<_>>();
        if let Some((low, high)) = self.f64_bits_for(target) {
            names.push(low);
            names.push(high);
        }
        for index in 0..self.stack.len() {
            if names
                .iter()
                .any(|name| self.stack[index].mentions_identifier(name))
            {
                let value = self.stack[index].clone();
                self.stack[index] = self.materialize(&value);
            }
        }
    }
    fn internal(&self, message: &str) -> CompileError {
        CompileError::new(
            ErrorKind::Internal,
            format!(
                "wasm2asm: internal code generation error in function {}: {message}",
                self.function_index
            ),
        )
    }
}

fn rename_module_functions(source: &str, function_names: &[String]) -> String {
    let mut counts = function_names
        .iter()
        .map(|name| (name.as_str(), 0usize))
        .collect::<BTreeMap<_, _>>();
    visit_javascript_identifiers(source, |start, end| {
        if let Some(count) = counts.get_mut(&source[start..end]) {
            *count += 1;
        }
    });
    let mut ranked = function_names
        .iter()
        .enumerate()
        .filter(|(_, name)| counts[name.as_str()] != 0)
        .collect::<Vec<_>>();
    ranked.sort_by(|(left_index, left), (right_index, right)| {
        counts[right.as_str()]
            .cmp(&counts[left.as_str()])
            .then_with(|| left_index.cmp(right_index))
    });
    let replacements = ranked
        .into_iter()
        .enumerate()
        .map(|(rank, (_, name))| (name.as_str(), function_ident(rank)))
        .collect::<BTreeMap<_, _>>();

    let mut output = String::with_capacity(source.len());
    let mut copied = 0usize;
    visit_javascript_identifiers(source, |start, end| {
        if let Some(replacement) = replacements.get(&source[start..end]) {
            output.push_str(&source[copied..start]);
            output.push_str(replacement);
            copied = end;
        }
    });
    if copied == 0 {
        return source.into();
    }
    output.push_str(&source[copied..]);
    output
}

fn visit_javascript_identifiers(source: &str, mut visit: impl FnMut(usize, usize)) {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if matches!(bytes[index], b'\'' | b'"') {
            let quote = bytes[index];
            index += 1;
            while index < bytes.len() && bytes[index] != quote {
                index += if bytes[index] == b'\\' { 2 } else { 1 };
            }
            index = index.saturating_add(1).min(bytes.len());
            continue;
        }
        if !bytes[index].is_ascii_alphabetic() && !matches!(bytes[index], b'_' | b'$') {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while bytes
            .get(index)
            .is_some_and(|byte| is_js_identifier_byte(*byte))
        {
            index += 1;
        }
        visit(start, index);
    }
}

fn identifier_is_read(source: &str, name: &str) -> bool {
    let mut read = false;
    visit_javascript_identifiers(source, |start, end| {
        if read || &source[start..end] != name {
            return;
        }
        let suffix = &source[end..];
        if !suffix.starts_with('=') || suffix.starts_with("==") {
            read = true;
        }
    });
    read
}

fn eliminate_write_only_i64_high_multiply(source: &str) -> String {
    let bytes = source.as_bytes();
    let mut removals = Vec::new();
    let mut from = 0usize;
    while let Some(relative) = source[from..].find("=$m(") {
        let equals = from + relative;
        let mut start = equals;
        while start > 0 && is_js_identifier_byte(bytes[start - 1]) {
            start -= 1;
        }
        let name = &source[start..equals];
        let boundary = start == 0 || matches!(bytes[start - 1], b';' | b'{' | b'}');
        let open = equals + 3;
        let Some(close) = matching_delimiter(source, open, b'(', b')') else {
            break;
        };
        let end = if bytes.get(close + 1..close + 3) == Some(b"|0") {
            close + 3
        } else {
            close + 1
        };
        if boundary
            && is_js_identifier(name)
            && bytes.get(end) == Some(&b';')
            && !identifier_is_read(source, name)
        {
            removals.push((start, end + 1));
        }
        from = end;
    }
    if removals.is_empty() {
        return source.into();
    }
    let mut output = String::with_capacity(source.len());
    let mut copied = 0usize;
    for (start, end) in removals {
        output.push_str(&source[copied..start]);
        copied = end;
    }
    output.push_str(&source[copied..]);
    output
}

fn optimize_labeled_early_exits(source: &str) -> String {
    let mut output = String::with_capacity(source.len());
    let mut copied = 0usize;
    while let Some((label_start, label_end, open, close)) = find_labeled_block(source, copied) {
        output.push_str(&source[copied..label_start]);
        let label = &source[label_start..label_end];
        let mut body = optimize_labeled_early_exits(&source[open + 1..close]);
        while let Some((start, replacement)) = best_early_exit_rewrite(&body, label) {
            body.truncate(start);
            body.push_str(&replacement);
        }
        if label_is_referenced(&body, label) {
            output.push_str(label);
            output.push(':');
        }
        output.push('{');
        output.push_str(&body);
        output.push('}');
        copied = close + 1;
    }
    if copied == 0 {
        return source.into();
    }
    output.push_str(&source[copied..]);
    output
}

fn find_labeled_block(source: &str, from: usize) -> Option<(usize, usize, usize, usize)> {
    let bytes = source.as_bytes();
    let mut index = from;
    while index < bytes.len() {
        if !bytes[index].is_ascii_alphabetic() && !matches!(bytes[index], b'_' | b'$') {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while bytes
            .get(index)
            .is_some_and(|byte| is_js_identifier_byte(*byte))
        {
            index += 1;
        }
        if bytes.get(index..index + 2) == Some(b":{")
            && (start == 0
                || bytes
                    .get(start - 1)
                    .is_some_and(|byte| matches!(byte, b'{' | b'}' | b';')))
        {
            let open = index + 1;
            if let Some(close) = matching_delimiter(source, open, b'{', b'}') {
                return Some((start, index, open, close));
            }
        }
    }
    None
}

fn matching_delimiter(source: &str, open: usize, opening: u8, closing: u8) -> Option<usize> {
    let bytes = source.as_bytes();
    let mut depth = 0usize;
    let mut index = open;
    while index < bytes.len() {
        if matches!(bytes[index], b'\'' | b'"') {
            let quote = bytes[index];
            index += 1;
            while index < bytes.len() && bytes[index] != quote {
                index += if bytes[index] == b'\\' { 2 } else { 1 };
            }
        } else if bytes[index] == opening {
            depth += 1;
        } else if bytes[index] == closing {
            depth -= 1;
            if depth == 0 {
                return Some(index);
            }
        }
        index += 1;
    }
    None
}

fn top_level_conditional_breaks(source: &str, label: &str) -> Vec<(usize, usize, usize)> {
    let bytes = source.as_bytes();
    let mut found = Vec::new();
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b'i' if depth == 0 && source[index..].starts_with("if(") => {
                let Some(condition_end) = matching_delimiter(source, index + 2, b'(', b')') else {
                    break;
                };
                let statement = format!("break {label};");
                let statement_start = condition_end + 1;
                if source[statement_start..].starts_with(&statement) {
                    found.push((index, condition_end, statement_start + statement.len()));
                    index = statement_start + statement.len();
                    continue;
                }
            }
            _ => {}
        }
        index += 1;
    }
    found
}

fn top_level_conditional_block_breaks(
    source: &str,
    label: &str,
) -> Vec<(usize, usize, usize, usize)> {
    let bytes = source.as_bytes();
    let mut found = Vec::new();
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            b'i' if depth == 0 && source[index..].starts_with("if(") => {
                let Some(condition_end) = matching_delimiter(source, index + 2, b'(', b')') else {
                    break;
                };
                let block_open = condition_end + 1;
                if bytes.get(block_open) != Some(&b'{') {
                    index = condition_end + 1;
                    continue;
                }
                let Some(block_close) = matching_delimiter(source, block_open, b'{', b'}') else {
                    break;
                };
                if source[block_close + 1..].starts_with("else") {
                    index = block_close + 1;
                    continue;
                }
                let body_start = block_open + 1;
                let mut body_end = block_close;
                if bytes.get(body_end.wrapping_sub(1)) == Some(&b';') {
                    body_end -= 1;
                }
                let statement = format!("break {label}");
                if source[body_start..body_end].ends_with(&statement) {
                    let break_start = body_end - statement.len();
                    if break_start == body_start
                        || bytes
                            .get(break_start - 1)
                            .is_some_and(|byte| matches!(byte, b';' | b'{' | b'}'))
                    {
                        found.push((index, condition_end, block_close + 1, break_start));
                    }
                }
                index = block_close + 1;
                continue;
            }
            _ => {}
        }
        index += 1;
    }
    found
}

fn best_early_exit_rewrite(source: &str, label: &str) -> Option<(usize, String)> {
    let mut best = None;
    for (start, condition_end, end) in top_level_conditional_breaks(source, label) {
        let suffix = &source[end..];
        if suffix.is_empty() {
            continue;
        }
        let condition = &source[start + 3..condition_end];
        let replacement = format!("if({}){{{suffix}}}", negate_condition(condition));
        if replacement.len() < source.len() - start {
            best = Some((start, replacement));
        }
    }
    for (start, condition_end, end, break_start) in
        top_level_conditional_block_breaks(source, label)
    {
        let condition = &source[start + 3..condition_end];
        let body_start = condition_end + 2;
        let prefix = &source[body_start..break_start];
        let suffix = &source[end..];
        let replacement = if suffix.is_empty() {
            format!("if({condition}){{{prefix}}}")
        } else if prefix.is_empty() {
            format!("if({}){{{suffix}}}", negate_condition(condition))
        } else {
            format!("if({condition}){{{prefix}}}else{{{suffix}}}")
        };
        if replacement.len() < source.len() - start
            && best
                .as_ref()
                .is_none_or(|(best_start, _)| start > *best_start)
        {
            best = Some((start, replacement));
        }
    }
    best
}

fn negate_condition(condition: &str) -> String {
    if let Some(value) = condition.strip_prefix('!')
        && (is_js_atom(value) || is_fully_parenthesized(value))
    {
        return value.into();
    }
    if is_js_atom(condition) || is_fully_parenthesized(condition) {
        return format!("!{condition}");
    }
    format!("!({condition})")
}

fn label_is_referenced(source: &str, label: &str) -> bool {
    let mut referenced = false;
    visit_generated_identifiers(source, |start, end| {
        if &source[start..end] == label && label_reference_context(source, start) {
            referenced = true;
        }
    });
    referenced
}

fn rename_function_labels(source: &str) -> String {
    let mut labels = Vec::new();
    visit_generated_identifiers(source, |start, end| {
        if label_declaration_context(source, start, end) {
            labels.push(source[start..end].to_string());
        }
    });
    if labels.is_empty() {
        return source.into();
    }

    let label_set = labels.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let mut counts = labels
        .iter()
        .cloned()
        .map(|label| (label, 0usize))
        .collect::<BTreeMap<_, _>>();
    visit_generated_identifiers(source, |start, end| {
        let label = &source[start..end];
        if label_set.contains(label)
            && (label_declaration_context(source, start, end)
                || label_reference_context(source, start))
        {
            *counts.get_mut(label).unwrap() += 1;
        }
    });
    labels.sort_by(|left, right| counts[right].cmp(&counts[left]));
    let replacements = labels
        .iter()
        .enumerate()
        .map(|(index, label)| (label.as_str(), label_ident(index)))
        .collect::<BTreeMap<_, _>>();

    let mut output = String::with_capacity(source.len());
    let mut copied = 0usize;
    visit_generated_identifiers(source, |start, end| {
        let label = &source[start..end];
        if let Some(replacement) = replacements.get(label)
            && (label_declaration_context(source, start, end)
                || label_reference_context(source, start))
        {
            output.push_str(&source[copied..start]);
            output.push_str(replacement);
            copied = end;
        }
    });
    if copied == 0 {
        return source.into();
    }
    output.push_str(&source[copied..]);
    output
}

fn visit_generated_identifiers(source: &str, mut visit: impl FnMut(usize, usize)) {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if !bytes[index].is_ascii_alphabetic() && !matches!(bytes[index], b'_' | b'$') {
            index += 1;
            continue;
        }
        let start = index;
        index += 1;
        while bytes
            .get(index)
            .is_some_and(|byte| is_js_identifier_byte(*byte))
        {
            index += 1;
        }
        visit(start, index);
    }
}

fn label_declaration_context(source: &str, start: usize, end: usize) -> bool {
    let bytes = source.as_bytes();
    let previous = start
        .checked_sub(1)
        .and_then(|index| bytes.get(index))
        .copied();
    if !previous.is_some_and(|byte| matches!(byte, b'{' | b'}' | b';'))
        || bytes.get(end) != Some(&b':')
    {
        return false;
    }
    matches!(bytes.get(end + 1), Some(b'{'))
        || source[end + 1..].starts_with("for(")
        || source[end + 1..].starts_with("if(")
}

fn label_reference_context(source: &str, start: usize) -> bool {
    let bytes = source.as_bytes();
    let mut end = start;
    while end != 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut begin = end;
    while begin != 0 && is_js_identifier_byte(bytes[begin - 1]) {
        begin -= 1;
    }
    matches!(&source[begin..end], "break" | "continue")
}

fn rename_function_locals(source: &str, identifiers: &[String], reserved: &[String]) -> String {
    if identifiers.len() < 2 {
        return source.into();
    }
    let mut counts = identifiers
        .iter()
        .cloned()
        .map(|identifier| (identifier, 0usize))
        .collect::<BTreeMap<_, _>>();
    visit_local_identifiers(source, |identifier| {
        if let Some(count) = counts.get_mut(identifier) {
            *count += 1;
        }
    });
    let mut ranked = identifiers.iter().enumerate().collect::<Vec<_>>();
    ranked.sort_by(|(left_index, left), (right_index, right)| {
        counts[*right]
            .cmp(&counts[*left])
            .then_with(|| left_index.cmp(right_index))
    });
    let reserved = reserved.iter().map(String::as_str).collect::<BTreeSet<_>>();
    let mut replacements = BTreeMap::new();
    let mut candidate = 0usize;
    for (_, identifier) in ranked {
        let replacement = loop {
            let replacement = compact_index(candidate);
            candidate += 1;
            if !reserved.contains(replacement.as_str()) {
                break replacement;
            }
        };
        replacements.insert(identifier.as_str(), replacement);
    }

    let mut output = String::with_capacity(source.len());
    let mut copied = 0usize;
    visit_local_identifier_spans(source, |start, end| {
        if let Some(replacement) = replacements.get(&source[start..end]) {
            output.push_str(&source[copied..start]);
            output.push_str(replacement);
            copied = end;
        }
    });
    if copied == 0 {
        return source.into();
    }
    output.push_str(&source[copied..]);
    output
}

fn visit_local_identifiers<'a>(source: &'a str, mut visit: impl FnMut(&'a str)) {
    visit_local_identifier_spans(source, |start, end| visit(&source[start..end]));
}

fn visit_local_identifier_spans(source: &str, mut visit: impl FnMut(usize, usize)) {
    let bytes = source.as_bytes();
    let mut index = 0usize;
    while index < bytes.len() {
        if !bytes[index].is_ascii_alphabetic() && !matches!(bytes[index], b'_' | b'$') {
            index += 1;
            continue;
        }
        let start = index;
        if index != 0 && (bytes[index - 1].is_ascii_digit() || bytes[index - 1] == b'.') {
            index += 1;
            continue;
        }
        index += 1;
        while bytes
            .get(index)
            .is_some_and(|byte| is_js_identifier_byte(*byte))
        {
            index += 1;
        }
        if local_identifier_context_is_eligible(source, start, index) {
            visit(start, index);
        }
    }
}

fn local_identifier_context_is_eligible(source: &str, start: usize, end: usize) -> bool {
    let bytes = source.as_bytes();
    let previous = (0..start)
        .rev()
        .find(|&index| !bytes[index].is_ascii_whitespace())
        .map(|index| bytes[index]);
    let next = (end..bytes.len())
        .find(|&index| !bytes[index].is_ascii_whitespace())
        .map(|index| bytes[index]);
    let key_or_label =
        next == Some(b':') && previous.is_none_or(|byte| matches!(byte, b'{' | b'}' | b';' | b','));
    if previous == Some(b'.') || key_or_label {
        return false;
    }
    let mut previous_end = start;
    while previous_end != 0 && bytes[previous_end - 1].is_ascii_whitespace() {
        previous_end -= 1;
    }
    let mut previous_start = previous_end;
    while previous_start != 0 && is_js_identifier_byte(bytes[previous_start - 1]) {
        previous_start -= 1;
    }
    !matches!(&source[previous_start..previous_end], "break" | "continue")
}
fn generated_call_arguments<'a>(
    source: &'a str,
    start: usize,
    name: &str,
) -> Option<(usize, Vec<&'a str>)> {
    if start != 0 && is_js_identifier_byte(source.as_bytes()[start - 1]) {
        return None;
    }
    let open = start + name.len();
    if source.as_bytes().get(open) != Some(&b'(') {
        return None;
    }
    let bytes = source.as_bytes();
    let mut arguments = Vec::new();
    let mut argument_start = open + 1;
    let mut depth = 0usize;
    let mut index = argument_start;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' if depth == 0 => {
                arguments.push(&source[argument_start..index]);
                return Some((index + 1, arguments));
            }
            b')' => depth -= 1,
            b',' if depth == 0 => {
                arguments.push(&source[argument_start..index]);
                argument_start = index + 1;
            }
            _ => {}
        }
        index += 1;
    }
    None
}

fn static_memory_key(arguments: &[&str]) -> Option<(i32, u8)> {
    let address = arguments.first()?.strip_suffix("|0")?.parse().ok()?;
    let code = arguments.get(1)?.parse().ok()?;
    Some((address, code))
}

fn static_store_key(arguments: &[&str]) -> Option<(i32, u8)> {
    if arguments.len() != 4 {
        return None;
    }
    let key = static_memory_key(arguments)?;
    if !matches!(key.1, 1 | 6 | 7 | 8) && arguments[3] != "0" {
        return None;
    }
    Some(key)
}

fn optimize_static_memory_accesses(compiled: &mut [CompiledFunction]) -> String {
    let mut load_counts = BTreeMap::<(i32, u8), usize>::new();
    let mut store_counts = BTreeMap::<(i32, u8), usize>::new();
    for function in compiled.iter() {
        for (start, _) in function.code.match_indices("l(") {
            if let Some((_, arguments)) = generated_call_arguments(&function.code, start, "l")
                && arguments.len() == 2
                && let Some(key) = static_memory_key(&arguments)
            {
                *load_counts.entry(key).or_default() += 1;
            }
        }
        for (start, _) in function.code.match_indices("st(") {
            if let Some((_, arguments)) = generated_call_arguments(&function.code, start, "st")
                && let Some(key) = static_store_key(&arguments)
            {
                *store_counts.entry(key).or_default() += 1;
            }
        }
    }

    let mut load_counts = load_counts.into_iter().collect::<Vec<_>>();
    load_counts.sort_by(|(left_key, left_count), (right_key, right_count)| {
        right_count.cmp(left_count).then(left_key.cmp(right_key))
    });
    let mut store_counts = store_counts.into_iter().collect::<Vec<_>>();
    store_counts.sort_by(|(left_key, left_count), (right_key, right_count)| {
        right_count.cmp(left_count).then(left_key.cmp(right_key))
    });

    let mut definitions = String::new();
    let mut load_helpers = BTreeMap::new();
    for (key @ (address, code), count) in load_counts {
        let name = format!("$c{}", short_index(load_helpers.len()));
        let original = format!("l({address}|0,{code})");
        let replacement = format!("{name}()");
        let definition = format!("function {name}(){{return l({address}|0,{code})|0}}");
        if original.len().saturating_sub(replacement.len()) * count > definition.len() {
            definitions.push_str(&definition);
            load_helpers.insert(key, name);
        }
    }

    let mut store_helpers = BTreeMap::new();
    for (key @ (address, code), count) in store_counts {
        let name = format!("$d{}", short_index(store_helpers.len()));
        let (original_fixed, replacement_fixed, definition) = if matches!(code, 1 | 6 | 7 | 8) {
            (
                format!("st({address}|0,{code},,)"),
                format!("{name}(,)"),
                format!("function {name}(a,b){{a=a|0;b=b|0;st({address}|0,{code}|0,a|0,b|0)}}"),
            )
        } else {
            (
                format!("st({address}|0,{code},,0)"),
                format!("{name}()"),
                format!("function {name}(a){{a=a|0;st({address}|0,{code}|0,a|0,0|0)}}"),
            )
        };
        if original_fixed.len().saturating_sub(replacement_fixed.len()) * count > definition.len() {
            definitions.push_str(&definition);
            store_helpers.insert(key, name);
        }
    }

    if load_helpers.is_empty() && store_helpers.is_empty() {
        return definitions;
    }
    for function in compiled {
        let source = &function.code;
        let mut output = String::with_capacity(source.len());
        let mut copied = 0usize;
        let mut index = 0usize;
        while index < source.len() {
            let load = source[index..].starts_with("l(").then(|| {
                generated_call_arguments(source, index, "l").and_then(|(end, arguments)| {
                    (arguments.len() == 2)
                        .then(|| static_memory_key(&arguments))
                        .flatten()
                        .and_then(|key| {
                            load_helpers
                                .get(&key)
                                .map(|name| (end, format!("{name}()")))
                        })
                })
            });
            let store = source[index..].starts_with("st(").then(|| {
                generated_call_arguments(source, index, "st").and_then(|(end, arguments)| {
                    let key = static_store_key(&arguments)?;
                    let name = store_helpers.get(&key)?;
                    let replacement = if matches!(key.1, 1 | 6 | 7 | 8) {
                        format!("{name}({},{})", arguments[2], arguments[3])
                    } else {
                        format!("{name}({})", arguments[2])
                    };
                    Some((end, replacement))
                })
            });
            let replacement = load.flatten().or_else(|| store.flatten());
            if let Some((end, replacement)) = replacement {
                output.push_str(&source[copied..index]);
                output.push_str(&replacement);
                copied = end;
                index = end;
            } else {
                index += 1;
            }
        }
        if copied != 0 {
            output.push_str(&source[copied..]);
            function.code = output;
        }
    }
    definitions
}

fn compiled_function_body(code: &str) -> Option<&str> {
    let open = code.find('{')?;
    let close = code.rfind('}')?;
    if open < close {
        Some(&code[open + 1..close])
    } else {
        None
    }
}
fn call_name_before(body: &str, open: usize) -> Option<&str> {
    let bytes = body.as_bytes();
    let mut end = open;
    while end > 0 && bytes[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && (is_js_identifier_byte(bytes[start - 1]) || bytes[start - 1] == b'.') {
        start -= 1;
    }
    let name = &body[start..end];
    (!name.is_empty()).then_some(name)
}
fn literal_requires_constant_call_argument(name: Option<&str>, argument: usize) -> bool {
    match name {
        Some("l2" | "lf2" | "st2" | "sf2") => matches!(argument, 1 | 2),
        Some("l" | "lf" | "st" | "sf") => argument == 1,
        _ => false,
    }
}
fn numeric_literals(body: &str) -> Vec<NumericLiteral> {
    let bytes = body.as_bytes();
    let mut literals = Vec::new();
    let mut calls = Vec::new();
    let mut quote = None;
    let mut escaped = false;
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if let Some(delimiter) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == delimiter {
                quote = None;
            }
            index += 1;
            continue;
        }
        if matches!(byte, b'\'' | b'"' | b'`') {
            quote = Some(byte);
            index += 1;
            continue;
        }
        match byte {
            b'(' => {
                calls.push((call_name_before(body, index), 0));
                index += 1;
                continue;
            }
            b',' => {
                if let Some((_, argument)) = calls.last_mut() {
                    *argument += 1;
                }
                index += 1;
                continue;
            }
            b')' => {
                calls.pop();
                index += 1;
                continue;
            }
            _ => {}
        }
        if !byte.is_ascii_digit()
            || index > 0 && is_js_identifier_byte(bytes[index - 1])
            || index + 1 < bytes.len() && is_js_identifier_byte(bytes[index + 1])
        {
            index += 1;
            continue;
        }
        let start = index;
        while index < bytes.len() && bytes[index].is_ascii_digit() {
            index += 1;
        }
        let end = index;
        if start > 0 && bytes[start - 1] == b'.' || end < bytes.len() && bytes[end] == b'.' {
            continue;
        }
        let prefix = &body[..start];
        let case_label = prefix
            .rfind("case ")
            .is_some_and(|case| prefix.rfind(':').is_none_or(|colon| colon < case));
        let must_literal = calls
            .iter()
            .any(|(name, argument)| literal_requires_constant_call_argument(*name, *argument));
        literals.push(NumericLiteral {
            start,
            end,
            case_label,
            must_literal,
        });
    }
    literals
}
fn literal_text<'a>(body: &'a str, literal: &NumericLiteral) -> &'a str {
    &body[literal.start..literal.end]
}
fn numeric_template_key(body: &str, literals: &[NumericLiteral]) -> String {
    let mut key = String::with_capacity(body.len());
    let mut previous = 0;
    for literal in literals {
        key.push_str(&body[previous..literal.start]);
        if literal.case_label || literal.must_literal {
            key.push_str(literal_text(body, literal));
        } else {
            key.push('#');
        }
        previous = literal.end;
    }
    key.push_str(&body[previous..]);
    key
}
fn render_numeric_template_body(
    body: &str,
    literals: &[NumericLiteral],
    differing: &[usize],
) -> String {
    let mut rendered = String::with_capacity(body.len());
    let mut previous = 0;
    for (index, literal) in literals.iter().enumerate() {
        rendered.push_str(&body[previous..literal.start]);
        if let Some(parameter) = differing.iter().position(|&candidate| candidate == index) {
            write!(rendered, "c{parameter}").unwrap();
        } else {
            rendered.push_str(literal_text(body, literal));
        }
        previous = literal.end;
    }
    rendered.push_str(&body[previous..]);
    rendered
}

fn type_compatible(a: ValType, b: ValType) -> bool {
    a == b || matches!((a, b), (ValType::FuncRef(_), ValType::FuncRef(_)))
}
fn component_decl_type(ty: ValType, _name: &str) -> ValType {
    match ty {
        ValType::I64 | ValType::V128 | ValType::FuncRef(_) => ValType::I32,
        other => other,
    }
}
fn value_component_type(value: &Value, _component: &str) -> ValType {
    match value {
        Value::I32(_) | Value::I64(_, _) | Value::Ref(_) | Value::V128(_) => ValType::I32,
        Value::F32(_) => ValType::F32,
        Value::F64(_) => ValType::F64,
    }
}
fn component_type(values: &[Value], name: &str) -> Option<ValType> {
    for value in values {
        for component in value.components() {
            if component == name {
                return Some(value_component_type(value, component));
            }
        }
    }
    None
}

fn named_value(ty: ValType, base: &str) -> Value {
    match ty {
        ValType::I32 => Value::I32(base.into()),
        ValType::I64 => Value::I64(format!("{base}L"), format!("{base}H")),
        ValType::F32 => Value::F32(base.into()),
        ValType::F64 => Value::F64(base.into()),
        ValType::FuncRef(_) => Value::Ref(base.into()),
        ValType::V128 => Value::V128(std::array::from_fn(|i| format!("{base}{i}"))),
    }
}
fn compact_memory_address(lo: &str, offset: u64) -> String {
    if offset == 0 {
        compact_i32(lo)
    } else {
        compact_operand(lo)
    }
}
fn local_ident(index: usize) -> String {
    compact_index(index)
}
fn parameter_values(types: &[ValType]) -> Vec<Value> {
    types
        .iter()
        .enumerate()
        .map(|(i, &ty)| named_value(ty, &local_ident(i)))
        .collect()
}
fn flatten_names(values: &[Value]) -> Vec<String> {
    values
        .iter()
        .flat_map(|v| v.components().into_iter().map(str::to_string))
        .collect()
}
fn flatten_call_arguments(values: &[Value], external: bool) -> Vec<String> {
    values
        .iter()
        .flat_map(|value| {
            value.components().into_iter().map(move |component| {
                if !external {
                    return component.to_string();
                }
                match value_component_type(value, component) {
                    ValType::F32 | ValType::F64 => compact_float_argument(component),
                    _ => compact_external_i32_argument(component),
                }
            })
        })
        .collect()
}
fn compact_external_i32_argument(value: &str) -> String {
    let value = strip_redundant_atom_parentheses(value.trim());
    if value.ends_with("|0") {
        value.into()
    } else if is_js_atom(value) {
        format!("{value}|0")
    } else {
        format!("({value})|0")
    }
}

fn zero_literal(ty: ValType) -> &'static str {
    match ty {
        ValType::F32 => "F(0)",
        ValType::F64 => "0.0",
        _ => "0",
    }
}
fn coerce(ty: ValType, value: &str) -> String {
    let value = strip_redundant_atom_parentheses(value.trim());
    match ty {
        ValType::I32 | ValType::I64 | ValType::FuncRef(_) | ValType::V128 => {
            if value.parse::<i32>().is_ok()
                || value.ends_with("|0")
                || is_signed_i32_expression(value)
            {
                value.to_string()
            } else {
                format!("{value}|0")
            }
        }
        ValType::F32 => {
            if value.starts_with("F(") {
                value.to_string()
            } else {
                format!("F({value})")
            }
        }
        ValType::F64 => {
            if value.starts_with(['+', '-']) {
                value.to_string()
            } else {
                format!("+{value}")
            }
        }
    }
}
fn coerce_assignment(ty: ValType, value: &str) -> String {
    let value = strip_redundant_atom_parentheses(value.trim());
    if is_js_identifier(value) {
        return value.into();
    }
    if matches!(
        ty,
        ValType::I32 | ValType::I64 | ValType::FuncRef(_) | ValType::V128
    ) && let Some(value) = value.strip_suffix("|0")
    {
        let value = strip_redundant_atom_parentheses(value);
        if is_js_identifier(value) || value.parse::<i32>().is_ok() {
            return value.into();
        }
    }
    coerce(ty, value)
}
fn emit_param_coercions(out: &mut String, values: &[Value]) {
    for value in values {
        for component in value.components() {
            write!(
                out,
                "{component}={};",
                coerce(value_component_type(value, component), component)
            )
            .unwrap();
        }
    }
}
fn emit_var_value(out: &mut String, target: &Value, source: &str) {
    match target {
        Value::I64(lo, hi) => {
            write!(out, "var {lo}=({source}.low)|0,{hi}=({source}.high)|0;").unwrap()
        }
        Value::V128(_) => {}
        _ => {
            let name = target.components()[0];
            write!(
                out,
                "var {name}={};",
                coerce(value_component_type(target, name), source)
            )
            .unwrap();
        }
    }
}
fn emit_var_value_from_value(out: &mut String, target: &Value, value: &Value) {
    for (name, source) in target.components().iter().zip(value.components()) {
        write!(out, "var {name}={source};").unwrap();
    }
}
fn emit_return_value(out: &mut String, value: &Value) {
    match value {
        Value::I64(lo, hi) => write!(
            out,
            "$q0={};return {};",
            coerce(ValType::I32, hi),
            coerce(ValType::I32, lo)
        )
        .unwrap(),
        Value::F32(x) => write!(out, "return {};", coerce(ValType::F32, x)).unwrap(),
        Value::F64(x) => write!(out, "return {};", coerce(ValType::F64, x)).unwrap(),
        Value::I32(x) | Value::Ref(x) => {
            write!(out, "return {};", coerce(ValType::I32, x)).unwrap()
        }
        Value::V128(_) => out.push_str("X();"),
    }
}
fn emit_set_from_single_argument(out: &mut String, value: &Value, arg: &str) {
    match value {
        Value::I64(_, _) => out.push_str("X();"),
        Value::F32(x) => write!(out, "{x}=F({arg});").unwrap(),
        Value::F64(x) => write!(out, "{x}=+{arg};").unwrap(),
        Value::I32(x) | Value::Ref(x) => write!(out, "{x}={arg}|0;").unwrap(),
        Value::V128(_) => out.push_str("X();"),
    }
}
fn emit_direct_js_return(out: &mut String, results: &[ValType], call: &str, external: bool) {
    if results.is_empty() {
        write!(out, "{call};return;").unwrap()
    } else {
        match results[0] {
            ValType::F32 if external => write!(out, "return F(+{call});").unwrap(),
            ValType::F32 => write!(out, "return {};", coerce(ValType::F32, call)).unwrap(),
            ValType::F64 => write!(out, "return {};", coerce(ValType::F64, call)).unwrap(),
            _ => write!(out, "return {call}|0;").unwrap(),
        }
    }
}
fn emit_primary_export_value(
    out: &mut String,
    ty: Option<ValType>,
    name: &str,
    high_slot: Option<usize>,
) {
    match ty {
        Some(ValType::I64) => write!(out, "[{name},q[{}]()]", high_slot.unwrap()).unwrap(),
        _ => out.push_str(name),
    }
}

fn const_value(expr: &ConstExpr, globals: &[Value]) -> Result<Value, CompileError> {
    Ok(match expr {
        ConstExpr::I32(v) => Value::I32(v.to_string()),
        ConstExpr::I64(v) => i64_const(*v),
        ConstExpr::F32(v) => Value::F32(float32_literal(*v)),
        ConstExpr::F64(v) => Value::F64(float64_literal(*v)),
        ConstExpr::GlobalGet(i) => globals
            .get(*i as usize)
            .ok_or_else(|| {
                CompileError::new(ErrorKind::Internal, "wasm2asm: invalid const global index")
            })?
            .clone(),
        ConstExpr::RefNull => Value::Ref("0".into()),
        ConstExpr::RefFunc(i) => Value::Ref((i + 1).to_string()),
    })
}
fn i64_const(value: i64) -> Value {
    Value::I64(
        (value as u32 as i32).to_string(),
        ((value >> 32) as i32).to_string(),
    )
}
fn float32_literal(bits: u32) -> String {
    let value = f32::from_bits(bits);
    if value.is_nan() {
        "F(Na)".into()
    } else if value == f32::INFINITY {
        "F(In)".into()
    } else if value == f32::NEG_INFINITY {
        "F(-In)".into()
    } else if bits == 0x80000000 {
        "F(-0.0)".into()
    } else {
        format!("F({:?})", value)
    }
}
fn float64_literal(bits: u64) -> String {
    let value = f64::from_bits(bits);
    if value.is_nan() {
        "Na".into()
    } else if value == f64::INFINITY {
        "In".into()
    } else if value == f64::NEG_INFINITY {
        "-In".into()
    } else if bits == 0x8000000000000000 {
        "-0.0".into()
    } else {
        format!("{:?}", value)
    }
}
fn return_slot_types(results: &[ValType]) -> Vec<ValType> {
    let mut out = Vec::new();
    match results.first() {
        Some(ValType::I64) => out.push(ValType::I32),
        Some(ValType::V128) => out.extend([ValType::I32; 3]),
        _ => {}
    }
    for &ty in results.iter().skip(1) {
        match ty {
            ValType::I64 => {
                out.push(ValType::I32);
                out.push(ValType::I32)
            }
            ValType::V128 => out.extend([ValType::I32; 4]),
            other => out.push(component_decl_type(other, "")),
        }
    }
    out
}

fn return_slot_class(ty: ValType) -> usize {
    match ty {
        ValType::I32 => 0,
        ValType::F32 => 1,
        ValType::F64 => 2,
        _ => unreachable!("return slot components are scalar asm.js types"),
    }
}

fn return_slot_layout(module: &Module) -> ([usize; 3], Vec<Value>) {
    let mut counts = [0; 3];
    for ty in &module.types {
        let mut signature_counts = [0; 3];
        for slot_ty in return_slot_types(&ty.results) {
            signature_counts[return_slot_class(slot_ty)] += 1;
        }
        for class in 0..3 {
            counts[class] = counts[class].max(signature_counts[class]);
        }
    }

    let mut slots = Vec::with_capacity(counts.iter().sum());
    for (class, (ty, prefix)) in [
        (ValType::I32, "$v"),
        (ValType::F32, "$f"),
        (ValType::F64, "$d"),
    ]
    .into_iter()
    .enumerate()
    {
        for index in 0..counts[class] {
            slots.push(named_value(ty, &format!("{prefix}{}", short_index(index))));
        }
    }
    (counts, slots)
}

fn return_slot_indices(results: &[ValType], counts: [usize; 3]) -> Vec<usize> {
    let bases = [0, counts[0], counts[0] + counts[1]];
    let mut used = [0; 3];
    return_slot_types(results)
        .into_iter()
        .map(|ty| {
            let class = return_slot_class(ty);
            let index = bases[class] + used[class];
            used[class] += 1;
            index
        })
        .collect()
}
fn emit_return_slot_assignment(out: &mut String, slot: &Value, value: &Value, component: &str) {
    let target = slot.components()[0];
    let expression = match (
        value_component_type(slot, target),
        value_component_type(value, component),
    ) {
        (ValType::F64, ValType::I32) => format!("+(({component})|0)"),
        (ValType::F64, _) => format!("+({component})"),
        (target_type, _) => coerce(target_type, &format!("({component})")),
    };
    write!(out, "{target}={expression};").unwrap();
}

fn return_slot_expression(slot: &Value, ty: ValType) -> String {
    coerce(
        component_decl_type(ty, slot.components()[0]),
        slot.components()[0],
    )
}

fn emit_return_abi(out: &mut String, values: &[Value], slots: &[Value]) {
    if values.is_empty() {
        out.push_str("return;");
        return;
    }
    let mut slot = 0usize;
    match &values[0] {
        Value::I64(_, hi) => {
            emit_return_slot_assignment(out, &slots[slot], &values[0], hi);
            slot += 1
        }
        Value::V128(components) => {
            for component in components.iter().skip(1) {
                emit_return_slot_assignment(out, &slots[slot], &values[0], component);
                slot += 1
            }
        }
        _ => {}
    }
    for value in values.iter().skip(1) {
        for component in value.components() {
            emit_return_slot_assignment(out, &slots[slot], value, component);
            slot += 1;
        }
    }
    match &values[0] {
        Value::I32(x) | Value::Ref(x) => {
            write!(out, "return {};", coerce(ValType::I32, x)).unwrap()
        }
        Value::I64(lo, _) => write!(out, "return {};", coerce(ValType::I32, lo)).unwrap(),
        Value::F32(x) => write!(out, "return {};", coerce(ValType::F32, x)).unwrap(),
        Value::F64(x) => write!(out, "return {};", coerce(ValType::F64, x)).unwrap(),
        Value::V128(components) => {
            write!(out, "return {};", coerce(ValType::I32, &components[0])).unwrap()
        }
    }
}
fn call_result_values(
    results: &[ValType],
    call: &str,
    slots: &[Value],
    body: &mut String,
    primary: Value,
    external: bool,
) -> Result<Vec<Value>, CompileError> {
    let name = primary.components()[0].to_string();
    let result_expression = if external && results[0] == ValType::F32 {
        format!("+{call}")
    } else {
        call.to_string()
    };
    write!(
        body,
        "{name}={};",
        coerce(value_component_type(&primary, &name), &result_expression)
    )
    .unwrap();
    let mut out = Vec::new();
    let mut slot = 0usize;
    match results[0] {
        ValType::I64 => {
            out.push(Value::I64(
                name,
                return_slot_expression(&slots[0], ValType::I32),
            ));
            slot = 1
        }
        ValType::V128 => {
            out.push(Value::V128([
                name,
                return_slot_expression(&slots[0], ValType::I32),
                return_slot_expression(&slots[1], ValType::I32),
                return_slot_expression(&slots[2], ValType::I32),
            ]));
            slot = 3
        }
        _ => out.push(primary),
    }
    for &ty in results.iter().skip(1) {
        let value = match ty {
            ValType::I64 => {
                let value = Value::I64(
                    return_slot_expression(&slots[slot], ValType::I32),
                    return_slot_expression(&slots[slot + 1], ValType::I32),
                );
                slot += 2;
                value
            }
            ValType::V128 => {
                let value = Value::V128(std::array::from_fn(|index| {
                    return_slot_expression(&slots[slot + index], ValType::I32)
                }));
                slot += 4;
                value
            }
            ValType::I32 => {
                let value = Value::I32(return_slot_expression(&slots[slot], ValType::I32));
                slot += 1;
                value
            }
            ValType::FuncRef(_) => {
                let value = Value::Ref(return_slot_expression(&slots[slot], ValType::I32));
                slot += 1;
                value
            }
            ValType::F32 => {
                let value = Value::F32(return_slot_expression(&slots[slot], ValType::F32));
                slot += 1;
                value
            }
            ValType::F64 => {
                let value = Value::F64(return_slot_expression(&slots[slot], ValType::F64));
                slot += 1;
                value
            }
        };
        out.push(value)
    }
    Ok(out)
}

fn expect_i64(v: Value) -> Result<(String, String), CompileError> {
    match v {
        Value::I64(a, b) => Ok((a, b)),
        _ => Err(CompileError::new(
            ErrorKind::Internal,
            "wasm2asm: expected i64",
        )),
    }
}
fn expect_v128(v: Value) -> Result<[String; 4], CompileError> {
    match v {
        Value::V128(a) => Ok(a),
        _ => Err(CompileError::new(
            ErrorKind::Internal,
            "wasm2asm: expected v128",
        )),
    }
}
fn expect_f32(v: Value) -> Result<String, CompileError> {
    match v {
        Value::F32(a) => Ok(a),
        _ => Err(CompileError::new(
            ErrorKind::Internal,
            "wasm2asm: expected f32",
        )),
    }
}
fn expect_f64(v: Value) -> Result<String, CompileError> {
    match v {
        Value::F64(a) => Ok(a),
        _ => Err(CompileError::new(
            ErrorKind::Internal,
            "wasm2asm: expected f64",
        )),
    }
}
fn expect_float(v: Value) -> Result<String, CompileError> {
    match v {
        Value::F32(a) | Value::F64(a) => Ok(a),
        _ => Err(CompileError::new(
            ErrorKind::Internal,
            "wasm2asm: expected float",
        )),
    }
}
fn expect_i32_pair(a: Value, b: Value) -> Result<(String, String), CompileError> {
    Ok((a.i32_expr()?.into(), b.i32_expr()?.into()))
}
fn is_js_identifier(value: &str) -> bool {
    let mut chars = value.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || matches!(first, '_' | '$')) && chars.all(is_js_identifier_char)
}
fn is_js_identifier_char(value: char) -> bool {
    value.is_ascii_alphanumeric() || matches!(value, '_' | '$')
}
fn reusable_i32_expression(value: &str) -> bool {
    let value = strip_redundant_atom_parentheses(value.trim());
    is_js_identifier(value)
        || value.parse::<i32>().is_ok()
        || value.strip_suffix("|0").is_some_and(is_js_identifier)
}
fn nonzero_guard_dominates(body: &str, value: &str) -> bool {
    let value = strip_redundant_atom_parentheses(value.trim());
    let name = value.strip_suffix("|0").unwrap_or(value);
    if !is_js_identifier(name) {
        return false;
    }
    let guard = format!("if(({value})==0)X();");
    let Some(guard_start) = body.rfind(&guard) else {
        return false;
    };
    let suffix = &body[guard_start + guard.len()..];
    if suffix.bytes().any(|byte| matches!(byte, b'{' | b'}')) {
        return false;
    }
    let mut assigned = false;
    visit_javascript_identifiers(suffix, |start, end| {
        if &suffix[start..end] == name
            && suffix[end..].starts_with('=')
            && !suffix[end..].starts_with("==")
        {
            assigned = true;
        }
    });
    !assigned
}

fn is_js_callee(value: &str) -> bool {
    !value.is_empty() && value.split('.').all(is_js_identifier)
}
fn is_js_call(value: &str) -> bool {
    let Some(open) = value.find('(') else {
        return false;
    };
    if !value.ends_with(')') || !is_js_callee(&value[..open]) {
        return false;
    }
    let mut depth = 0usize;
    for (index, byte) in value.as_bytes()[open..].iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return open + index + 1 == value.len();
                }
            }
            _ => {}
        }
    }
    false
}
fn is_fully_parenthesized(value: &str) -> bool {
    if !value.starts_with('(') || !value.ends_with(')') {
        return false;
    }
    let mut depth = 0usize;
    for (index, byte) in value.as_bytes().iter().enumerate() {
        match byte {
            b'(' => depth += 1,
            b')' => {
                depth = depth.saturating_sub(1);
                if depth == 0 && index + 1 != value.len() {
                    return false;
                }
            }
            _ => {}
        }
    }
    depth == 0
}
fn is_js_atom(value: &str) -> bool {
    let value = value.trim();
    if is_js_identifier(value) || is_js_call(value) || is_fully_parenthesized(value) {
        return true;
    }
    if let Some(value) = value.strip_prefix(['+', '-']) {
        return is_js_atom(value);
    }
    !value.is_empty()
        && value
            .chars()
            .all(|value| value.is_ascii_digit() || matches!(value, '.' | 'e' | 'E'))
}
fn strip_redundant_atom_parentheses(mut value: &str) -> &str {
    while is_fully_parenthesized(value) {
        let inner = &value[1..value.len() - 1];
        if !is_js_atom(inner) {
            break;
        }
        value = inner;
    }
    value
}

fn compact_operand(value: &str) -> String {
    let value = strip_redundant_atom_parentheses(value.trim());
    if value.starts_with(['+', '-']) {
        format!("({value})")
    } else if is_js_atom(value) {
        value.into()
    } else {
        format!("({value})")
    }
}
fn compact_i32(value: &str) -> String {
    let value = strip_redundant_atom_parentheses(value.trim());
    if value.ends_with("|0") || is_signed_i32_expression(value) {
        value.into()
    } else if is_js_atom(value) {
        format!("{value}|0")
    } else {
        format!("({value})|0")
    }
}

fn is_i32_zero(value: &str) -> bool {
    matches!(value.trim(), "0" | "0|0" | "(0|0)")
}
fn i32_literal(value: &str) -> Option<i32> {
    let value = strip_redundant_atom_parentheses(value.trim());
    value
        .strip_suffix("|0")
        .unwrap_or(value)
        .parse::<i32>()
        .ok()
}

fn is_unsigned_u16(value: &str) -> bool {
    let value = strip_redundant_atom_parentheses(value.trim());
    i32_literal(value).is_some_and(|value| (value as u32) <= u16::MAX as u32)
        || value.ends_with("&65535")
        || value.starts_with("65535&")
}

fn is_signed_i32_expression(value: &str) -> bool {
    let value = value.trim();
    if (value.starts_with("U(") || value.starts_with("C(")) && is_js_call(value) {
        return true;
    }
    let value = if is_fully_parenthesized(value) {
        &value[1..value.len() - 1]
    } else {
        value
    };
    let bytes = value.as_bytes();
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b'&' | b'^' if depth == 0 => return true,
            b'|' if depth == 0
                && bytes.get(index.wrapping_sub(1)) != Some(&b'|')
                && bytes.get(index + 1) != Some(&b'|') =>
            {
                return true;
            }
            b'<' if depth == 0 && bytes.get(index + 1) == Some(&b'<') => return true,
            b'>' if depth == 0
                && bytes.get(index + 1) == Some(&b'>')
                && bytes.get(index + 2) != Some(&b'>') =>
            {
                return true;
            }
            _ => {}
        }
        index += 1;
    }
    false
}

fn condition_syntax(value: &str) -> String {
    if is_fully_parenthesized(value) {
        value.into()
    } else {
        format!("({value})")
    }
}
fn compact_condition(value: &str) -> String {
    let value = value.trim();
    let Some(parenthesized) = value.strip_suffix("|0") else {
        return value.into();
    };
    let inner = if is_fully_parenthesized(parenthesized) {
        &parenthesized[1..parenthesized.len() - 1]
    } else {
        parenthesized
    };
    for (operator, negate) in [
        ("==0", true),
        ("!=0", false),
        ("==(0|0)", true),
        ("!=(0|0)", false),
    ] {
        if let Some(operand) = inner.strip_suffix(operator) {
            if let Some(identifier) = condition_identifier(operand) {
                return if negate {
                    format!("!{identifier}")
                } else {
                    identifier.into()
                };
            }
            return if negate {
                format!("!{}", condition_syntax(operand))
            } else {
                operand.into()
            };
        }
    }
    let bytes = inner.as_bytes();
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b'=' | b'!' if depth == 0 && bytes.get(index + 1) == Some(&b'=') => {
                return compact_condition_literals(inner);
            }
            b'<' if depth == 0 && bytes.get(index + 1) != Some(&b'<') => {
                return compact_condition_literals(inner);
            }
            b'>' if depth == 0 && bytes.get(index + 1) != Some(&b'>') => {
                return compact_condition_literals(inner);
            }
            _ => {}
        }
        index += 1;
    }
    value.into()
}

fn coerced_identifier(value: &str) -> Option<&str> {
    let identifier = value.strip_prefix('(')?.strip_suffix("|0)")?;
    is_js_identifier(identifier).then_some(identifier)
}

fn condition_identifier(value: &str) -> Option<&str> {
    is_js_identifier(value)
        .then_some(value)
        .or_else(|| coerced_identifier(value))
}

fn compact_condition_literals(value: &str) -> String {
    let bytes = value.as_bytes();
    let mut output = String::with_capacity(value.len());
    let mut copied = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] != b'(' {
            index += 1;
            continue;
        }
        let mut end = index + 1;
        if bytes.get(end) == Some(&b'-') {
            end += 1;
        }
        let digits = end;
        while bytes.get(end).is_some_and(u8::is_ascii_digit) {
            end += 1;
        }
        if end == digits || bytes.get(end..end + 3) != Some(b"|0)") {
            index += 1;
            continue;
        }
        let literal = &value[index + 1..end];
        if literal.parse::<i32>().is_err() {
            index += 1;
            continue;
        }
        output.push_str(&value[copied..index]);
        output.push_str(literal);
        copied = end + 3;
        index = copied;
    }
    if copied == 0 {
        return value.into();
    }
    output.push_str(&value[copied..]);
    output
}
fn compact_compare_operand(value: &str, unsigned: bool) -> String {
    if unsigned {
        format!("{}>>>0", compact_unsigned_i32_operand(value))
    } else {
        format!("({})", compact_i32(value))
    }
}

fn compact_unsigned_i32_operand(value: &str) -> String {
    let value = strip_redundant_atom_parentheses(value.trim());
    if is_js_identifier(value) {
        return value.into();
    }
    if let Some(parenthesized) = value.strip_suffix("|0")
        && is_fully_parenthesized(parenthesized)
    {
        return parenthesized.into();
    }
    compact_operand(value)
}
fn compact_float_argument(value: &str) -> String {
    let value = strip_redundant_atom_parentheses(value.trim());
    if value.starts_with('+') {
        value.into()
    } else if is_js_atom(value) {
        format!("+{value}")
    } else {
        format!("+({value})")
    }
}

fn i32_bin(a: Value, b: Value, op: &str) -> Result<Value, CompileError> {
    let (a, b) = expect_i32_pair(a, b)?;
    let bitwise = matches!(op, "&" | "|" | "^" | "<<" | ">>");
    let a = compact_nested_i32_operand(&a, bitwise);
    let b = compact_nested_i32_operand(&b, bitwise);
    let expression = format!("{a}{op}{b}");
    Ok(Value::I32(if bitwise {
        expression
    } else {
        format!("{expression}|0")
    }))
}

fn compact_nested_i32_operand(value: &str, bitwise: bool) -> String {
    let value = value.trim();
    let Some(parenthesized) = value.strip_suffix("|0") else {
        return compact_operand(value);
    };
    if !is_fully_parenthesized(parenthesized) {
        return compact_operand(value);
    }
    let inner = &parenthesized[1..parenthesized.len() - 1];
    if nested_i32_coercion_is_redundant(inner, bitwise) {
        parenthesized.into()
    } else {
        compact_operand(value)
    }
}

fn nested_i32_coercion_is_redundant(value: &str, bitwise: bool) -> bool {
    let bytes = value.as_bytes();
    let mut depth = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        match bytes[index] {
            b'(' => depth += 1,
            b')' => depth = depth.saturating_sub(1),
            b'+' if depth == 0 => return true,
            b'-' if depth == 0 && index != 0 => return true,
            b'>' if bitwise
                && depth == 0
                && bytes.get(index + 1) == Some(&b'>')
                && bytes.get(index + 2) == Some(&b'>') =>
            {
                return true;
            }
            _ => {}
        }
        index += 1;
    }
    false
}
fn cmp_i32(a: Value, b: Value, op: &str, unsigned: bool) -> Result<Value, CompileError> {
    let (a, b) = expect_i32_pair(a, b)?;
    Ok(Value::I32(format!(
        "{}{op}{}|0",
        compact_compare_operand(&a, unsigned),
        compact_compare_operand(&b, unsigned)
    )))
}
fn cmp_float(a: Value, b: Value, op: &str) -> Result<Value, CompileError> {
    Ok(Value::I32(format!(
        "{}{op}{}|0",
        compact_operand(&expect_float(a)?),
        compact_operand(&expect_float(b)?)
    )))
}
fn float_bin(a: Value, b: Value, op: &str, f32_: bool) -> Result<Value, CompileError> {
    let a = expect_float(a)?;
    let b = expect_float(b)?;
    Ok(if f32_ {
        Value::F32(format!(
            "F({}{op}{})",
            compact_operand(&a),
            compact_operand(&b)
        ))
    } else {
        Value::F64(format!(
            "+({}{op}{})",
            compact_operand(&a),
            compact_operand(&b)
        ))
    })
}
fn float_helper(a: Value, b: Value, name: &str, f32_: bool) -> Result<Value, CompileError> {
    let a = expect_float(a)?;
    let b = expect_float(b)?;
    let a = compact_float_argument(&a);
    let b = compact_float_argument(&b);
    Ok(if f32_ {
        Value::F32(format!("F(+{name}({a},{b}))"))
    } else {
        Value::F64(format!("+{name}({a},{b})"))
    })
}
fn i64_compare(a: Value, b: Value, op: BinaryOp) -> Result<Value, CompileError> {
    let (al, ah) = expect_i64(a)?;
    let (bl, bh) = expect_i64(b)?;
    let al_literal = i32_literal(&al);
    let ah_literal = i32_literal(&ah);
    let bl_literal = i32_literal(&bl);
    let bh_literal = i32_literal(&bh);
    let a_zero = al_literal == Some(0) && ah_literal == Some(0);
    let b_zero = bl_literal == Some(0) && bh_literal == Some(0);
    let al = format!("({})", compact_external_i32_argument(&al));
    let ah = format!("({})", compact_external_i32_argument(&ah));
    let bl = format!("({})", compact_external_i32_argument(&bl));
    let bh = format!("({})", compact_external_i32_argument(&bh));
    let expression = if b_zero {
        match op {
            BinaryOp::I64Eq => format!("((({al}|{ah})==0)|0)"),
            BinaryOp::I64Ne => format!("((({al}|{ah})!=0)|0)"),
            BinaryOp::I64LtS => format!("(({ah}<0)|0)"),
            BinaryOp::I64LtU => "0".into(),
            BinaryOp::I64GtS => format!("((({ah}>0)|(({ah}==0)&({al}!=0)))|0)"),
            BinaryOp::I64GtU => format!("((({al}|{ah})!=0)|0)"),
            BinaryOp::I64LeS => format!("((({ah}<0)|(({ah}==0)&({al}==0)))|0)"),
            BinaryOp::I64LeU => format!("((({al}|{ah})==0)|0)"),
            BinaryOp::I64GeS => format!("(({ah}>=0)|0)"),
            BinaryOp::I64GeU => "1".into(),
            _ => unreachable!(),
        }
    } else if a_zero {
        match op {
            BinaryOp::I64Eq => format!("((({bl}|{bh})==0)|0)"),
            BinaryOp::I64Ne => format!("((({bl}|{bh})!=0)|0)"),
            BinaryOp::I64LtS => format!("((({bh}>0)|(({bh}==0)&({bl}!=0)))|0)"),
            BinaryOp::I64LtU => format!("((({bl}|{bh})!=0)|0)"),
            BinaryOp::I64GtS => format!("(({bh}<0)|0)"),
            BinaryOp::I64GtU => "0".into(),
            BinaryOp::I64LeS => format!("(({bh}>=0)|0)"),
            BinaryOp::I64LeU => "1".into(),
            BinaryOp::I64GeS => format!("((({bh}<0)|(({bh}==0)&({bl}==0)))|0)"),
            BinaryOp::I64GeU => format!("((({bl}|{bh})==0)|0)"),
            _ => unreachable!(),
        }
    } else if bl_literal == Some(0) && !matches!(op, BinaryOp::I64Eq | BinaryOp::I64Ne) {
        match op {
            BinaryOp::I64LtS => format!("(({ah}<{bh})|0)"),
            BinaryOp::I64LtU => format!("((({ah}>>>0)<({bh}>>>0))|0)"),
            BinaryOp::I64GtS => format!("((({ah}>{bh})|(({ah}=={bh})&({al}!=0)))|0)"),
            BinaryOp::I64GtU => format!("(((({ah}>>>0)>({bh}>>>0))|(({ah}=={bh})&({al}!=0)))|0)"),
            BinaryOp::I64LeS => format!("((({ah}<{bh})|(({ah}=={bh})&({al}==0)))|0)"),
            BinaryOp::I64LeU => format!("(((({ah}>>>0)<({bh}>>>0))|(({ah}=={bh})&({al}==0)))|0)"),
            BinaryOp::I64GeS => format!("(({ah}>={bh})|0)"),
            BinaryOp::I64GeU => format!("((({ah}>>>0)>=({bh}>>>0))|0)"),
            _ => unreachable!(),
        }
    } else if bl_literal == Some(-1) && !matches!(op, BinaryOp::I64Eq | BinaryOp::I64Ne) {
        match op {
            BinaryOp::I64LtS => format!("((({ah}<{bh})|(({ah}=={bh})&({al}!=-1)))|0)"),
            BinaryOp::I64LtU => format!("(((({ah}>>>0)<({bh}>>>0))|(({ah}=={bh})&({al}!=-1)))|0)"),
            BinaryOp::I64GtS => format!("(({ah}>{bh})|0)"),
            BinaryOp::I64GtU => format!("((({ah}>>>0)>({bh}>>>0))|0)"),
            BinaryOp::I64LeS => format!("(({ah}<={bh})|0)"),
            BinaryOp::I64LeU => format!("((({ah}>>>0)<=({bh}>>>0))|0)"),
            BinaryOp::I64GeS => format!("((({ah}>{bh})|(({ah}=={bh})&({al}==-1)))|0)"),
            BinaryOp::I64GeU => format!("(((({ah}>>>0)>({bh}>>>0))|(({ah}=={bh})&({al}==-1)))|0)"),
            _ => unreachable!(),
        }
    } else if al_literal == Some(0) && !matches!(op, BinaryOp::I64Eq | BinaryOp::I64Ne) {
        match op {
            BinaryOp::I64LtS => format!("((({ah}<{bh})|(({ah}=={bh})&({bl}!=0)))|0)"),
            BinaryOp::I64LtU => format!("(((({ah}>>>0)<({bh}>>>0))|(({ah}=={bh})&({bl}!=0)))|0)"),
            BinaryOp::I64GtS => format!("(({ah}>{bh})|0)"),
            BinaryOp::I64GtU => format!("((({ah}>>>0)>({bh}>>>0))|0)"),
            BinaryOp::I64LeS => format!("(({ah}<={bh})|0)"),
            BinaryOp::I64LeU => format!("((({ah}>>>0)<=({bh}>>>0))|0)"),
            BinaryOp::I64GeS => format!("((({ah}>{bh})|(({ah}=={bh})&({bl}==0)))|0)"),
            BinaryOp::I64GeU => format!("(((({ah}>>>0)>({bh}>>>0))|(({ah}=={bh})&({bl}==0)))|0)"),
            _ => unreachable!(),
        }
    } else if al_literal == Some(-1) && !matches!(op, BinaryOp::I64Eq | BinaryOp::I64Ne) {
        match op {
            BinaryOp::I64LtS => format!("(({ah}<{bh})|0)"),
            BinaryOp::I64LtU => format!("((({ah}>>>0)<({bh}>>>0))|0)"),
            BinaryOp::I64GtS => format!("((({ah}>{bh})|(({ah}=={bh})&({bl}!=-1)))|0)"),
            BinaryOp::I64GtU => format!("(((({ah}>>>0)>({bh}>>>0))|(({ah}=={bh})&({bl}!=-1)))|0)"),
            BinaryOp::I64LeS => format!("((({ah}<{bh})|(({ah}=={bh})&({bl}==-1)))|0)"),
            BinaryOp::I64LeU => format!("(((({ah}>>>0)<({bh}>>>0))|(({ah}=={bh})&({bl}==-1)))|0)"),
            BinaryOp::I64GeS => format!("(({ah}>={bh})|0)"),
            BinaryOp::I64GeU => format!("((({ah}>>>0)>=({bh}>>>0))|0)"),
            _ => unreachable!(),
        }
    } else {
        match op {
            BinaryOp::I64Eq => format!("(({al}=={bl})&({ah}=={bh}))|0"),
            BinaryOp::I64Ne => format!("(({al}!={bl})|({ah}!={bh}))|0"),
            BinaryOp::I64LtS => {
                format!("(({ah}<{bh})|(({ah}=={bh})&(({al}>>>0)<({bl}>>>0))))|0")
            }
            BinaryOp::I64LtU => {
                format!("((({ah}>>>0)<({bh}>>>0))|(({ah}=={bh})&(({al}>>>0)<({bl}>>>0))))|0")
            }
            BinaryOp::I64GtS => {
                format!("(({ah}>{bh})|(({ah}=={bh})&(({al}>>>0)>({bl}>>>0))))|0")
            }
            BinaryOp::I64GtU => {
                format!("((({ah}>>>0)>({bh}>>>0))|(({ah}=={bh})&(({al}>>>0)>({bl}>>>0))))|0")
            }
            BinaryOp::I64LeS => {
                format!("(({ah}<{bh})|(({ah}=={bh})&(({al}>>>0)<=({bl}>>>0))))|0")
            }
            BinaryOp::I64LeU => {
                format!("((({ah}>>>0)<({bh}>>>0))|(({ah}=={bh})&(({al}>>>0)<=({bl}>>>0))))|0")
            }
            BinaryOp::I64GeS => {
                format!("(({ah}>{bh})|(({ah}=={bh})&(({al}>>>0)>=({bl}>>>0))))|0")
            }
            BinaryOp::I64GeU => {
                format!("((({ah}>>>0)>({bh}>>>0))|(({ah}=={bh})&(({al}>>>0)>=({bl}>>>0))))|0")
            }
            _ => unreachable!(),
        }
    };
    Ok(Value::I32(expression))
}

fn load_code(op: LoadOp) -> u8 {
    use LoadOp::*;
    match op {
        I32 => 0,
        I64 => 1,
        F32 => 2,
        F64 => 3,
        I32_8S => 4,
        I32_8U => 5,
        I32_16S => 6,
        I32_16U => 7,
        I64_8S => 8,
        I64_8U => 9,
        I64_16S => 10,
        I64_16U => 11,
        I64_32S => 12,
        I64_32U => 13,
    }
}
fn store_code(op: StoreOp) -> u8 {
    use StoreOp::*;
    match op {
        I32 => 0,
        I64 => 1,
        F32 => 2,
        F64 => 3,
        I32_8 => 4,
        I32_16 => 5,
        I64_8 => 6,
        I64_16 => 7,
        I64_32 => 8,
    }
}
fn label_ident(index: usize) -> String {
    match index {
        0 => "$".into(),
        1 => "_".into(),
        _ => compact_index(index - 2),
    }
}

fn function_ident(index: usize) -> String {
    let mut candidate = 0;
    let mut remaining = index;
    loop {
        let name = function_name_candidate(candidate);
        if !is_reserved_function(&name) {
            if remaining == 0 {
                return name;
            }
            remaining -= 1;
        }
        candidate += 1;
    }
}

fn function_name_candidate(mut index: usize) -> String {
    const FIRST: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ";
    const REST: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789$_";
    if index == 0 {
        return "$".into();
    }
    if index == 1 {
        return "_".into();
    }
    index -= 2;
    if index < FIRST.len() {
        return char::from(FIRST[index]).into();
    }
    index -= FIRST.len();
    let mut suffix_len = 1usize;
    let mut suffix_space = REST.len();
    while index >= FIRST.len() * suffix_space {
        index -= FIRST.len() * suffix_space;
        suffix_len += 1;
        suffix_space *= REST.len();
    }
    let mut name = String::with_capacity(suffix_len + 1);
    name.push(char::from(FIRST[index / suffix_space]));
    let mut suffix = index % suffix_space;
    let mut divisor = suffix_space;
    for _ in 0..suffix_len {
        divisor /= REST.len();
        name.push(char::from(REST[suffix / divisor]));
        suffix %= divisor;
    }
    name
}
fn is_reserved_function(name: &str) -> bool {
    matches!(
        name,
        "AA" | "AB"
            | "B0"
            | "AC"
            | "C"
            | "DD"
            | "ED"
            | "F"
            | "G"
            | "GH"
            | "IC"
            | "IG"
            | "In"
            | "K"
            | "L"
            | "LI"
            | "LF"
            | "Ma"
            | "Mc"
            | "Mf"
            | "Ms"
            | "MS"
            | "N"
            | "Na"
            | "O"
            | "P"
            | "Q"
            | "R"
            | "RS"
            | "SI"
            | "SF"
            | "TS"
            | "U"
            | "V"
            | "VL"
            | "VS"
            | "W"
            | "X"
            | "Y"
    ) || name
        .strip_prefix('I')
        .or_else(|| name.strip_prefix('J'))
        .is_some_and(|suffix| {
            !suffix.is_empty() && suffix.bytes().all(|byte| byte.is_ascii_lowercase())
        })
}
fn js_ident(index: usize, upper: bool) -> String {
    let mut n = index;
    let first = if upper { b'A' } else { b'a' };
    let mut s = String::new();
    loop {
        s.push((first + (n % 26) as u8) as char);
        n /= 26;
        if n == 0 {
            break;
        }
        n -= 1;
    }
    s
}
fn short_index(index: usize) -> String {
    let mut n = index;
    let mut s = String::new();
    loop {
        s.push((b'a' + (n % 26) as u8) as char);
        n /= 26;
        if n == 0 {
            break;
        }
        n -= 1;
    }
    s
}
fn compact_index(index: usize) -> String {
    let mut candidate = 0;
    let mut remaining = index;
    loop {
        let name = js_ident(candidate, false);
        if !is_reserved_local(&name) {
            if remaining == 0 {
                return name;
            }
            remaining -= 1;
        }
        candidate += 1;
    }
}

fn is_reserved_local(name: &str) -> bool {
    matches!(
        name,
        "l" | "y"
            | "ct"
            | "pc"
            | "tr"
            | "ne"
            | "mn"
            | "mx"
            | "cs"
            | "rf"
            | "ri"
            | "rd"
            | "wr"
            | "lf"
            | "st"
            | "sf"
            | "vl"
            | "vs"
            | "as"
            | "do"
            | "if"
            | "in"
            | "of"
            | "for"
            | "let"
            | "new"
            | "try"
            | "var"
            | "case"
            | "else"
            | "enum"
            | "eval"
            | "false"
            | "null"
            | "this"
            | "true"
            | "void"
            | "with"
            | "await"
            | "break"
            | "catch"
            | "class"
            | "const"
            | "super"
            | "throw"
            | "while"
            | "yield"
            | "delete"
            | "export"
            | "import"
            | "public"
            | "static"
            | "arguments"
            | "interface"
            | "implements"
            | "package"
            | "private"
            | "protected"
    )
}
fn js_byte_string(bytes: &[u8]) -> String {
    let mut out = String::from("\"");
    for pair in bytes.chunks(2) {
        let unit = u16::from(pair[0]) << 8 | u16::from(*pair.get(1).unwrap_or(&0));
        match unit {
            0x22 => out.push_str("\\\""),
            0x5c => out.push_str("\\\\"),
            0x20..=0x7e => out.push(char::from_u32(u32::from(unit)).unwrap()),
            0x80..=0xd7ff | 0xe000..=0xffff if !matches!(unit, 0x2028 | 0x2029) => {
                out.push(char::from_u32(u32::from(unit)).unwrap());
            }
            _ => write!(out, "\\u{unit:04x}").unwrap(),
        }
    }
    out.push('"');
    out
}

fn js_string(value: &str) -> String {
    let mut out = String::from("\"");
    for c in value.chars() {
        match c {
            '\"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c if c < ' ' => write!(out, "\\u{:04x}", c as u32).unwrap(),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
fn instruction_offset(_: &str, _: &str) -> usize {
    0
}

const RUNTIME_HELPERS_PREFIX: &str = r#"
function X(){throw Error('wasm trap')}
function ct(x){x=x|0;if(!x)return 32;return 32-C((x&-x)-1)|0}
function pc(x){x=x|0;x=x-((x>>>1)&1431655765)|0;x=(x&858993459)+((x>>>2)&858993459)|0;return U((x+(x>>>4)&252645135),16843009)>>>24}
function tr(x){x=+x;return x<0?M.ceil(x):M.floor(x)}
function ne(x){x=+x;var f=M.floor(x),d=x-f;if(d<.5)return f;if(d>.5)return f+1;if(f%2){f=f+1;return f==0&&x<0?-0:f}return f==0&&x<0?-0:f}
function mn(a,b){a=+a;b=+b;if(a!=a||b!=b)return NaN;if(a==0&&b==0)return 1/a<0?a:b;return a<b?a:b}
function mx(a,b){a=+a;b=+b;if(a!=a||b!=b)return NaN;if(a==0&&b==0)return 1/a>0?a:b;return a>b?a:b}
var sb=new ArrayBuffer(8),sv=new DataView(sb);
function cs(a,b){a=+a;b=+b;sv.setFloat64(0,a,true);var h=sv.getUint32(4,true);sv.setFloat64(0,b,true);h=(h&2147483647)|(sv.getUint32(4,true)&2147483648);sv.setUint32(4,h,true);return +sv.getFloat64(0,true)}
function rf(x){x=+x;sv.setFloat32(0,x,true);return sv.getInt32(0,true)|0}
function ri(x){x=x|0;sv.setInt32(0,x,true);return +sv.getFloat32(0,true)}
function rd(x){x=+x;sv.setFloat64(0,x,true);hi=sv.getInt32(4,true)|0;return sv.getInt32(0,true)|0}
function wr(l,h){l=l|0;h=h|0;sv.setInt32(0,l,true);sv.setInt32(4,h,true);return +sv.getFloat64(0,true)}
function Y(x,u,z){x=+x;u=u|0;z=z|0;if(x!=x)return z?0:X();if(u){if(x<=-1)return z?0:X();if(x>=4294967296)return z?-1:X();return tr(x)|0}else{if(x<=-2147483649)return z?-2147483648:X();if(x>=2147483648)return z?2147483647:X();return tr(x)|0}}
function y(x,u,z){x=+x;u=u|0;z=z|0;var n=0,h=0,l=0;if(x!=x){if(!z)X();hi=0;return 0}if(u){if(x<=-1){if(!z)X();hi=0;return 0}if(x>=18446744073709551616){if(!z)X();hi=-1;return -1}x=tr(x);h=M.floor(x/4294967296);l=x-h*4294967296;hi=h|0;return l|0}else{if(x<-9223372036854775808){if(!z)X();hi=-2147483648;return 0}if(x>=9223372036854775808){if(!z)X();hi=2147483647;return -1}n=x<0;if(n)x=-x;x=tr(x);h=M.floor(x/4294967296);l=x-h*4294967296;l=l|0;h=h|0;if(n){l=(~l+1)|0;h=(~h+(l==0))|0}hi=h;return l}}
function ng(l,h){l=l|0;h=h|0;l=(~l+1)|0;hi=(~h+(l==0))|0;return l}
function ge(al,ah,bl,bh){al=al|0;ah=ah|0;bl=bl|0;bh=bh|0;return ((ah>>>0)>(bh>>>0)||ah==bh&&(al>>>0)>=(bl>>>0))|0}
function dv(al,ah,bl,bh,sg,rm){al=al|0;ah=ah|0;bl=bl|0;bh=bh|0;sg=sg|0;rm=rm|0;var nq=0,nr=0,i=0,ql=0,qh=0,rl=0,rh=0,bit=0,t=0;if(!(bl|bh))X();if(sg&&ah==-2147483648&&!al&&bh==-1&&bl==-1&&!rm)X();if(sg){nq=(ah<0)^(bh<0);nr=ah<0;if(ah<0){al=ng(al,ah)|0;ah=hi|0}if(bh<0){bl=ng(bl,bh)|0;bh=hi|0}}for(i=63;i>=0;i--){bit=i<32?(al>>>i)&1:(ah>>>(i-32))&1;rh=(rh<<1)|(rl>>>31);rl=(rl<<1)|bit;if(ge(rl,rh,bl,bh)){t=rl-bl|0;rh=(rh-bh-((rl>>>0)<(bl>>>0)))|0;rl=t;if(i<32)ql=ql|(1<<i);else qh=qh|(1<<(i-32))}}if(rm){if(nr){rl=ng(rl,rh)|0;rh=hi|0}hi=rh;return rl}if(nq){ql=ng(ql,qh)|0;qh=hi|0}hi=qh;return ql}

function W(o,al,ah,bl,bh){o=o|0;al=al|0;ah=ah|0;bl=bl|0;bh=bh|0;var l=0,h=0,n=0,a0=0,a1=0,a2=0,a3=0,b0=0,b1=0,b2=0,b3=0,c0=0,c1=0,c2=0,c3=0;if(o==0){l=al+bl|0;hi=ah+bh+((l>>>0)<(al>>>0))|0;return l}if(o==1){l=al-bl|0;hi=ah-bh-((al>>>0)<(bl>>>0))|0;return l}if(o==2){a0=al&65535;a1=al>>>16;a2=ah&65535;a3=ah>>>16;b0=bl&65535;b1=bl>>>16;b2=bh&65535;b3=bh>>>16;c0=a0*b0;c1=a1*b0+a0*b1+M.floor(c0/65536);c2=a2*b0+a1*b1+a0*b2+M.floor(c1/65536);c3=a3*b0+a2*b1+a1*b2+a0*b3+M.floor(c2/65536);l=(c0&65535)|((c1&65535)<<16);hi=(c2&65535)|((c3&65535)<<16);return l}if(o==3){hi=ah&bh;return al&bl}if(o==4){hi=ah|bh;return al|bl}if(o==5){hi=ah^bh;return al^bl}if(o>=11)return dv(al,ah,bl,bh,(o==11||o==13)|0,(o==13||o==14)|0)|0;n=bl&63;if(!n){hi=ah;return al}if(o==6){if(n<32){hi=(ah<<n)|(al>>>(32-n));return al<<n}hi=al<<(n-32);return 0}if(o==7){if(n<32){hi=ah>>n;return (al>>>n)|(ah<<(32-n))}hi=ah>>31;return ah>>(n-32)}if(o==8){if(n<32){hi=ah>>>n;return (al>>>n)|(ah<<(32-n))}hi=0;return ah>>>(n-32)}if(o==9){if(n<32){hi=(ah<<n)|(al>>>(32-n));return (al<<n)|(ah>>>(32-n))}n=n-32;hi=(al<<n)|(ah>>>(32-n));return (ah<<n)|(al>>>(32-n))}if(n<32){hi=(ah>>>n)|(al<<(32-n));return (al>>>n)|(ah<<(32-n))}n=n-32;hi=(al>>>n)|(ah<<(32-n));return (ah>>>n)|(al<<(32-n))}
"#;

const CHECKED_ADDRESS_HELPER: &str = r#"function AA(k,l,h,o,w){k=k|0;l=l|0;h=h|0;o=+o;w=w|0;var x=0;if(h||o>4294967295)X();x=(l>>>0)+o;if(x<0||x+w>s[k])X();return (a[k]+x)|0}"#;
const FAST_ADDRESS_HELPER: &str = r#"function AA(k,l,h,o,w){return (a[k|0]+(l>>>0)+o)|0}"#;

const RUNTIME_HELPERS_SUFFIX: &str = r#"
function l(a,t){a=a|0;t=t|0;var x=AA(0,a,0,0,t==1?8:t==6||t==7||t==10||t==11?2:t==4||t==5||t==8||t==9?1:4);if(t==1){hi=V.getInt32(x+4,true);return V.getUint32(x,true)|0}if(t==4||t==8)return V.getInt8(x)|0;if(t==5||t==9)return V.getUint8(x)|0;if(t==6||t==10)return V.getInt16(x,true)|0;if(t==7||t==11)return V.getUint16(x,true)|0;if(t==12)return V.getInt32(x,true)|0;if(t==13)return V.getUint32(x,true)|0;return V.getInt32(x,true)|0}
function lf(a,t){a=a|0;t=t|0;var x=AA(0,a,0,0,t==3?8:4);return t==3?V.getFloat64(x,true):V.getFloat32(x,true)}
function st(a,t,v,w){a=a|0;t=t|0;v=v|0;w=w|0;var x=AA(0,a,0,0,t==1?8:t==5||t==7?2:t==4||t==6?1:4);if(t==1){V.setInt32(x,v,true);V.setInt32(x+4,w,true)}else if(t==4||t==6)V.setInt8(x,v);else if(t==5||t==7)V.setInt16(x,v,true);else V.setInt32(x,v,true)}
function sf(a,t,v){a=a|0;t=t|0;v=+v;var x=AA(0,a,0,0,t==3?8:4);if(t==3)V.setFloat64(x,v,true);else V.setFloat32(x,v,true)}
function vl(a,i){a=a|0;i=i|0;var x=AA(0,a,0,0,16);return V.getInt32(x+i*4,true)|0}
function vs(a,v0,v1,v2,v3){a=a|0;v0=v0|0;v1=v1|0;v2=v2|0;v3=v3|0;var x=AA(0,a,0,0,16);V.setInt32(x,v0,true);V.setInt32(x+4,v1,true);V.setInt32(x+8,v2,true);V.setInt32(x+12,v3,true)}
function G(k,d){k=k|0;d=d|0;var old=0,add=0,ns=0,total=0,x=0,y=0,nb=null,nh=null;if(d<0)return -1;old=s[k]/p[k]|0;add=(d>>>0)*p[k];ns=s[k]+add;if(ns>4294967295||m[k]>=0&&ns>m[k])return -1;for(x=0;x<s.length;x++)total+=x==k?ns:s[x];if(total>4294967295)return -1;nb=new ArrayBuffer(total);nh=new Uint8Array(nb);for(x=0,y=0;x<s.length;x++){nh.set(new Uint8Array(B,a[x],s[x]),y);a[x]=y;y+=x==k?ns:s[x]}s[k]=ns;B=r.b=nb;V=new DataView(B);H=new Uint8Array(B);return old}

function AB(dm,sm,d,sr,n){dm=dm|0;sm=sm|0;d=d>>>0;sr=sr>>>0;n=n>>>0;var da=AA(dm,d,0,0,n),sa=AA(sm,sr,0,0,n);H.set(H.subarray(sa,sa+n),da)}
function AC(k,d,v,n){k=k|0;d=d>>>0;v=v|0;n=n>>>0;var x=AA(k,d,0,0,n);H.fill(v,x,x+n)}
function K(di,k,d,sr,n){di=di|0;k=k|0;d=d>>>0;sr=sr>>>0;n=n>>>0;var x=0,i=0,j=0,v='';v=D[di].b;if(D[di].x||sr+n>D[di].n)X();x=AA(k,d,0,0,n);for(i=0;i<n;i++){j=sr+i;H[x+i]=(j&1)?v.charCodeAt(j>>>1)&255:v.charCodeAt(j>>>1)>>>8}}
function N(x){x=x>>>0;if(x>=T.length)X();return T[x]|0}
function O(x,v){x=x>>>0;v=v|0;if(x>=T.length)X();T[x]=v;S[x]=v?Z[v]:-1}
function P(v,n){v=v|0;n=n>>>0;var o=T.length,i=0;if(r.l>=0&&o+n>r.l)return -1;for(i=0;i<n;i++){T.push(v);S.push(v?Z[v]:-1)}return o|0}
function Q(d,v,n){d=d>>>0;v=v|0;n=n>>>0;var i=0;if(d+n>T.length)X();for(i=0;i<n;i++){T[d+i]=v;S[d+i]=v?Z[v]:-1}}
function R(d,sr,n){d=d>>>0;sr=sr>>>0;n=n>>>0;var i=0;if(d+n>T.length||sr+n>T.length)X();if(d>sr&&d<sr+n)for(i=n-1;i>=0;i--){T[d+i]=T[sr+i];S[d+i]=S[sr+i]}else for(i=0;i<n;i++){T[d+i]=T[sr+i];S[d+i]=S[sr+i]}}
function L(e,d,sr,n){e=e|0;d=d>>>0;sr=sr>>>0;n=n>>>0;var i=0,v=0;if(E[e].x||sr+n>E[e].length||d+n>T.length)X();for(i=0;i<n;i++){v=E[e][sr+i]|0;T[d+i]=v;S[d+i]=v?Z[v]:-1}}
function LI(k,l,h,o,t){var x=AA(k,l,h,o,t==1?8:t==6||t==7||t==10||t==11?2:t==4||t==5||t==8||t==9?1:4);if(t==1){hi=V.getInt32(x+4,true);return V.getUint32(x,true)|0}if(t==4||t==8)return V.getInt8(x)|0;if(t==5||t==9)return V.getUint8(x)|0;if(t==6||t==10)return V.getInt16(x,true)|0;if(t==7||t==11)return V.getUint16(x,true)|0;if(t==12)return V.getInt32(x,true)|0;if(t==13)return V.getUint32(x,true)|0;return V.getInt32(x,true)|0}
function LF(k,l,h,o,t){var x=AA(k,l,h,o,t==3?8:4);return t==3?V.getFloat64(x,true):V.getFloat32(x,true)}
function SI(k,l,h,o,t,v,w){var x=AA(k,l,h,o,t==1?8:t==5||t==7?2:t==4||t==6?1:4);if(t==1){V.setInt32(x,v,true);V.setInt32(x+4,w,true)}else if(t==4||t==6)V.setInt8(x,v);else if(t==5||t==7)V.setInt16(x,v,true);else V.setInt32(x,v,true)}
function SF(k,l,h,o,t,v){var x=AA(k,l,h,o,t==3?8:4);if(t==3)V.setFloat64(x,v,true);else V.setFloat32(x,v,true)}
function MS(k){return s[k]/p[k]|0}
function VL(k,l,h,o,i){var x=AA(k,l,h,o,16);return V.getInt32(x+i*4,true)|0}
function VS(k,l,h,o,a,b,c,d){var x=AA(k,l,h,o,16);V.setInt32(x,a,true);V.setInt32(x+4,b,true);V.setInt32(x+8,c,true);V.setInt32(x+12,d,true)}
function DD(i){D[i].x=1}
function ED(i){E[i].x=1}
function TS(){return T.length|0}
function RS(v,t){return (v!=0&&Z[v]==t)|0}
"#;

const CHECKED_INDIRECT_HELPER: &str =
    r#"function IG(x,t){x=x>>>0;if(x>=T.length||!T[x]||S[x]!=t)X();return T[x]|0}"#;
const FAST_INDIRECT_HELPER: &str = r#"function IG(x,t){return T[x>>>0]|0}"#;
