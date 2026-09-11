use crate::common::{DifferentialCase, assert_differential};
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
fn bulk_memory_operations_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/bulk_memory_operations.wat");
    assert_differential(DifferentialCase::same(
        "bulk memory operations",
        wat,
        "(()=>{m.run();return Array.from(new Uint8Array(m.memory.buffer,0,40))})()",
    ));
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
