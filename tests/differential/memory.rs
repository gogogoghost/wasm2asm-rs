use crate::common::{DifferentialCase, assert_differential};
use wasm2asm::CompileOptions;

#[test]
fn memory_access_and_growth_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/memory_access_and_growth.wat");
    assert_differential(DifferentialCase {
        name: "memory access and growth",
        wat,
        native_expression: "(()=>{m.setup();var before=[m.i32loads(),m.i64loads().map(i64Pair),m.floats(),Array.from(new Uint8Array(m.memory.buffer,0,64))];var old=m.grow(1);return [before,old,m.memory.buffer.byteLength]})()",
        asm_expression: "(()=>{m.setup();var before=[m.i32loads(),m.i64loads(),m.floats(),Array.from(new Uint8Array(m.memory.buffer,0,64))];var old=m.grow(1);return [before,old,m.memory.buffer.byteLength]})()",
        native_imports: "{}",
        asm_imports: "{}",
        options: Default::default(),
    });
}
#[test]
fn fast_memory_access_and_growth_match_native_wasm_for_valid_inputs() {
    let wat = include_str!("../fixtures/differential/memory_access_and_growth.wat");
    let options = CompileOptions {
        preserve_traps: false,
        ..CompileOptions::default()
    };
    assert_differential(DifferentialCase {
        name: "fast memory access and growth",
        wat,
        native_expression: "(()=>{m.setup();var before=[m.i32loads(),m.i64loads().map(i64Pair),m.floats(),Array.from(new Uint8Array(m.memory.buffer,0,64))];var old=m.grow(1);return [before,old,m.memory.buffer.byteLength]})()",
        asm_expression: "(()=>{m.setup();var before=[m.i32loads(),m.i64loads(),m.floats(),Array.from(new Uint8Array(m.memory.buffer,0,64))];var old=m.grow(1);return [before,old,m.memory.buffer.byteLength]})()",
        native_imports: "{}",
        asm_imports: "{}",
        options,
    });
}

#[test]
fn bulk_memory_operations_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/bulk_memory_operations.wat");
    assert_differential(DifferentialCase::same(
        "bulk memory operations",
        wat,
        "(()=>{m.run();return Array.from(new Uint8Array(m.memory.buffer,0,40))})()",
    ));
}

#[test]
fn fast_bulk_memory_operations_match_native_wasm_for_valid_inputs() {
    let wat = include_str!("../fixtures/differential/bulk_memory_operations.wat");
    let options = CompileOptions {
        preserve_traps: false,
        ..CompileOptions::default()
    };
    assert_differential(DifferentialCase {
        name: "fast bulk memory operations",
        wat,
        native_expression: "(()=>{m.run();return Array.from(new Uint8Array(m.memory.buffer,0,40))})()",
        asm_expression: "(()=>{m.run();return Array.from(new Uint8Array(m.memory.buffer,0,40))})()",
        native_imports: "{}",
        asm_imports: "{}",
        options,
    });
}

#[test]
fn bulk_copy_preserves_overlap_and_atomic_bounds_failures() {
    let wat = include_str!("../fixtures/differential/bulk_memory_operations.wat");
    let valid = r#"
      m.run();
      var b=new Uint8Array(m.memory.buffer),rows=[];
      for (var [d,s,n] of [[9,8,5],[8,9,5],[0,0,40],[65536,65536,0],[65528,8,8]]) {
        m.copy(d,s,n);
        rows.push([Array.from(b.slice(0,40)),Array.from(b.slice(-8))]);
      }
    "#;
    let bounds = r#"
      for (var [d,s,n] of [[65535,0,2],[0,65535,2],[0,0,65537],
          [65537,0,0],[0,65537,0],[0,-1,1]]) {
        rows.push([trapped(()=>m.copy(d,s,n)),Array.from(b.slice(0,8)),Array.from(b.slice(-8))]);
      }
    "#;
    for preserve_traps in [true, false] {
        let bounds = if preserve_traps { bounds } else { "" };
        let expression = format!("(()=>{{{valid}{bounds}return rows}})()");
        assert_differential(DifferentialCase {
            name: "bulk copy overlap and atomic bounds failures",
            wat,
            native_expression: &expression,
            asm_expression: &expression,
            native_imports: "{}",
            asm_imports: "{}",
            options: CompileOptions {
                preserve_traps,
                ..Default::default()
            },
        });
    }
}
#[test]
fn imported_memory_matches_native_wasm() {
    let wat = include_str!("../fixtures/differential/imported_memory.wat");
    let imports = "{env:{memory:new WebAssembly.Memory({initial:1,maximum:2})}}";
    assert_differential(DifferentialCase {
        name: "imported memory",
        wat,
        native_expression: "[m.roundTrip(305419896),m.memory]",
        asm_expression: "[m.roundTrip(305419896),m.memory]",
        native_imports: imports,
        asm_imports: imports,
        options: Default::default(),
    });
}
#[test]
fn imported_memory_uses_actual_buffer_size() {
    let wat = r#"(module
      (import "env" "memory" (memory 1 3))
      (func (export "pages") (result i32) memory.size)
      (func (export "roundTrip") (param i32) (result i32)
        i32.const 131068 local.get 0 i32.store
        i32.const 131068 i32.load))"#;
    let imports = "{env:{memory:new WebAssembly.Memory({initial:3,maximum:3})}}";
    for preserve_traps in [true, false] {
        assert_differential(DifferentialCase {
            name: "actual imported memory size",
            wat,
            native_expression: "[m.pages(),m.roundTrip(-19088744)]",
            asm_expression: "[m.pages(),m.roundTrip(-19088744)]",
            native_imports: imports,
            asm_imports: imports,
            options: CompileOptions { preserve_traps, ..Default::default() },
        });
    }
}

#[test]
fn low_i64_load_projections_preserve_full_consumers_and_bounds() {
    let wat = r#"(module
      (memory 256 256)
      (func (export "store") (param i32 i64)
        local.get 0 local.get 1 i64.store)
      (func (export "low") (param i32) (result i32)
        local.get 0 i64.load i32.wrap_i64)
      (func (export "addLow") (param i32 i32) (result i32)
        local.get 0 i64.load local.get 1 i64.extend_i32_s i64.add i32.wrap_i64)
      (func (export "full") (param i32) (result i64)
        local.get 0 i64.load)
      (func (export "mixed") (param i32) (result i64) (local i64)
        i32.const 128 local.get 0 i64.load local.tee 1 i32.wrap_i64 i32.store
        local.get 1)
      (func (export "mixedArithmetic") (param i32) (result i64) (local i64)
        i32.const 128 local.get 0 i64.load i64.const 4294967297 i64.add
        local.tee 1 i32.wrap_i64 i32.store local.get 1))"#;
    for fast_aligned in [false, true] {
        let traps = if fast_aligned { "" } else {
            ",trapped(()=>m.low(16777212))"
        };
        let native_expression = format!("(()=>{{m.store(16,0xfedcba9876543210n);return [m.low(16),m.addLow(16,2147483647),i64Pair(m.full(16)),i64Pair(m.mixed(16)),i64Pair(m.mixedArithmetic(16)),m.low(128){traps}]}})()");
        let asm_expression = format!("(()=>{{m.store(16,1985229328,-19088744);var a=m.full(16),ah=m.getTempRet0(),b=m.mixed(16),bh=m.getTempRet0(),c=m.mixedArithmetic(16),ch=m.getTempRet0();return [m.low(16),m.addLow(16,2147483647),[a,ah],[b,bh],[c,ch],m.low(128){traps}]}})()");
        assert_differential(DifferentialCase {
            name: "low i64 load projections",
            wat,
            native_expression: &native_expression,
            asm_expression: &asm_expression,
            native_imports: "{}",
            asm_imports: "{}",
            options: CompileOptions {
                preserve_traps: !fast_aligned,
                assume_memory_alignment: fast_aligned,
                ..Default::default()
            },
        });
    }
}

#[test]
fn unaligned_integer_memory_preserves_bytes_signs_and_trap_atomicity() {
    let wat = include_str!("../fixtures/differential/unaligned_integer_memory.wat");
    let body = r#"
      var rows=[], end=16777216;
      function bytes(a) { return Array.from({length:8},(_,i)=>m.byte(a+i)); }
      for (var base of [128,end-15]) for (var offset=0;offset<8;offset++) {
        var a=base+offset;
        s64('store64',a);
        rows.push([bytes(a),m.read32(a),m.read16s(a),m.read16u(a),
          r64('read64',a),r64('read64_16s',a),r64('read64_16u',a),
          r64('read64_32s',a),r64('read64_32u',a)]);
        m.store16(a,0xface);
        rows.push([m.read16s(a),m.read16u(a),bytes(a)]);
        m.store32(a,0x80012345|0);
        rows.push([m.read32(a),bytes(a)]);
        s64('store64_16',a);
        rows.push([m.read16u(a),bytes(a)]);
        s64('store64_32',a);
        rows.push([r64('read64_32s',a),r64('read64_32u',a),bytes(a)]);
      }
    "#;
    let bounds = r#"
      s64('store64',end-8);
      rows.push(bytes(end-8));
      for (var action of [
        ()=>m.read64(end-7),()=>m.read32(end-3),()=>m.read16u(end-1),
        ()=>m.read32(-1),()=>m.read64(-2147483648),
        ()=>s64('store64',end-7),()=>m.store32(end-3,0),
        ()=>m.store16(end-1,0),()=>s64('store64_16',end-1),
        ()=>s64('store64_32',end-3)
      ]) rows.push([trapped(action),bytes(end-8)]);
    "#;
    let native_helpers = "function s64(n,a){m[n](a,0xfedcba9889abcdefn)}function r64(n,a){return i64Pair(m[n](a))}";
    let asm_helpers = "function s64(n,a){m[n](a,-1985229329,-19088744)}function r64(n,a){var v=m[n](a);return [v,m.getTempRet0()]}";
    for preserve_traps in [true, false] {
        for assume_memory_alignment in [false, true] {
            let bounds = if preserve_traps { bounds } else { "" };
            let native_expression = format!("(()=>{{{native_helpers}{body}{bounds}return rows}})()");
            let asm_expression = format!("(()=>{{{asm_helpers}{body}{bounds}return rows}})()");
            assert_differential(DifferentialCase {
                name: "unaligned integer bytes, signs and trap atomicity",
                wat,
                native_expression: &native_expression,
                asm_expression: &asm_expression,
                native_imports: "{}",
                asm_imports: "{}",
                options: CompileOptions {
                    preserve_traps,
                    assume_memory_alignment,
                    ..Default::default()
                },
            });
        }
    }
}

#[test]
fn direct_fixed_memory_access_matches_native_wasm() {
    let wat = include_str!("../fixtures/differential/direct_fixed_memory_access.wat");
    assert_differential(DifferentialCase::same(
        "direct fixed memory access",
        wat,
        "[m.run(),trapped(()=>m.oob())]",
    ));
}

#[test]
fn fast_direct_fixed_memory_matches_native_wasm_for_valid_inputs() {
    let wat = include_str!("../fixtures/differential/direct_fixed_memory_access.wat");
    let options = CompileOptions {
        preserve_traps: false,
        ..CompileOptions::default()
    };
    assert_differential(DifferentialCase {
        name: "fast direct fixed memory access",
        wat,
        native_expression: "m.run()",
        asm_expression: "m.run()",
        native_imports: "{}",
        asm_imports: "{}",
        options,
    });
}

#[test]
fn assumed_memory_alignment_respects_declared_hints() {
    let wat = include_str!("../fixtures/differential/direct_fixed_memory_access.wat");
    let options = CompileOptions {
        preserve_traps: false,
        assume_memory_alignment: true,
        ..CompileOptions::default()
    };
    assert_differential(DifferentialCase {
        name: "assumed memory alignment",
        wat,
        native_expression: "m.run()",
        asm_expression: "m.run()",
        native_imports: "{}",
        asm_imports: "{}",
        options,
    });
}

#[test]
fn memory64_access_and_growth_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/memory64_access_and_growth.wat");
    assert_differential(DifferentialCase {
        name: "memory64 access and growth",
        wat,
        native_expression: "(()=>{m.store(8n,305419896);var q=m.roundTrip(),before=m.load(8n),old=m.grow(1n),size=m.size();return [[q[0],i64Pair(q[1]),q[2],q[3]],before,i64Pair(old),i64Pair(size),m.memory.buffer.byteLength]})()",
        asm_expression: "(()=>{m.store(8,0,305419896);var q=m.roundTrip(),before=m.load(8,0),old=m.grow(1,0),oldHigh=m.getTempRet0(),size=m.size(),sizeHigh=m.getTempRet0();return [q,before,[old,oldHigh],[size,sizeHigh],m.memory.buffer.byteLength]})()",
        native_imports: "{}",
        asm_imports: "{}",
        options: Default::default(),
    });
}

#[test]
fn fast_memory64_matches_native_wasm_for_valid_inputs() {
    let wat = include_str!("../fixtures/differential/memory64_access_and_growth.wat");
    let options = CompileOptions {
        preserve_traps: false,
        ..CompileOptions::default()
    };
    assert_differential(DifferentialCase {
        name: "fast memory64 access and growth",
        wat,
        native_expression: "(()=>{m.store(8n,305419896);var q=m.roundTrip(),before=m.load(8n),old=m.grow(1n),size=m.size();return [[q[0],i64Pair(q[1]),q[2],q[3]],before,i64Pair(old),i64Pair(size),m.memory.buffer.byteLength]})()",
        asm_expression: "(()=>{m.store(8,0,305419896);var q=m.roundTrip(),before=m.load(8,0),old=m.grow(1,0),oldHigh=m.getTempRet0(),size=m.size(),sizeHigh=m.getTempRet0();return [q,before,[old,oldHigh],[size,sizeHigh],m.memory.buffer.byteLength]})()",
        native_imports: "{}",
        asm_imports: "{}",
        options,
    });
}

#[test]
fn multiple_memories_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/multiple_memories.wat");
    assert_differential(DifferentialCase {
        name: "multiple memories",
        wat,
        native_expression: "(()=>{var r=m.run();return [[r[0],i64Pair(r[1]),r[2],r[3],r[4],r[5]],Array.from(new Uint8Array(m.memory.buffer,0,56))]})()",
        asm_expression: "(()=>{return [m.run(),Array.from(new Uint8Array(m.memory.buffer,0,56))]})()",
        native_imports: "{}",
        asm_imports: "{}",
        options: Default::default(),
    });
}

#[test]
fn fast_multiple_memories_match_native_wasm_for_valid_inputs() {
    let wat = include_str!("../fixtures/differential/multiple_memories.wat");
    let options = CompileOptions {
        preserve_traps: false,
        ..CompileOptions::default()
    };
    assert_differential(DifferentialCase {
        name: "fast multiple memories",
        wat,
        native_expression: "(()=>{var r=m.run();return [[r[0],i64Pair(r[1]),r[2],r[3],r[4],r[5]],Array.from(new Uint8Array(m.memory.buffer,0,56))]})()",
        asm_expression: "(()=>{return [m.run(),Array.from(new Uint8Array(m.memory.buffer,0,56))]})()",
        native_imports: "{}",
        asm_imports: "{}",
        options,
    });
}
