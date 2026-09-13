use crate::common::{DifferentialCase, assert_differential};
use wasm2asm::{CompileOptions, Lowerings};
#[test]
fn table_and_indirect_call_operations_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/table_and_indirect_call_operations.wat");
    assert_differential(DifferentialCase::same(
        "table and indirect calls",
        wat,
        "[m.apply(0,41),m.apply(1,21),m.mutate()]",
    ));
}

#[test]
fn fast_indirect_calls_match_native_wasm_for_valid_inputs() {
    let wat = include_str!("../fixtures/differential/table_and_indirect_call_operations.wat");
    let options = CompileOptions {
        preserve_traps: false,
        ..CompileOptions::default()
    };
    assert_differential(DifferentialCase {
        name: "fast table and indirect calls",
        wat,
        native_expression: "[m.apply(0,41),m.apply(1,21),m.mutate()]",
        asm_expression: "[m.apply(0,41),m.apply(1,21),m.mutate()]",
        native_imports: "{}",
        asm_imports: "{}",
        options,
    });
}
#[test]
fn typed_function_reference_calls_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/typed_function_reference_calls.wat");
    let options = CompileOptions {
        lowerings: Lowerings::REFERENCES,
        ..CompileOptions::default()
    };
    assert_differential(DifferentialCase {
        name: "typed function reference calls",
        wat,
        native_expression: "[m.apply(-1),m.apply(41),m.checks()]",
        asm_expression: "[m.apply(-1),m.apply(41),m.checks()]",
        native_imports: "{}",
        asm_imports: "{}",
        options,
    });
}

#[test]
fn indirect_call_signatures_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/indirect_call_signatures.wat");
    assert_differential(DifferentialCase {
        name: "indirect call signatures",
        wat,
        native_expression: "(()=>{m.applyVoid(305419896);return [new DataView(m.memory.buffer).getInt32(0,true),m.applySingle(2.5),m.applyDouble(3.75),i64Pair(m.applyInternalWide(5)),i64Pair(m.applyImportedWide(5))]})()",
        asm_expression: "(()=>{m.applyVoid(305419896);var internal=m.applyInternalWide(5),internalHigh=m.getTempRet0(),imported=m.applyImportedWide(5),importedHigh=m.getTempRet0();return [new DataView(m.memory.buffer).getInt32(0,true),m.applySingle(2.5),m.applyDouble(3.75),[internal,internalHigh],[imported,importedHigh]]})()",
        native_imports: "{env:{wide:function(x){return BigInt(x)+8589934592n;}}}",
        asm_imports: "{env:{wide:function(x){return x|0;},getTempRet0:function(){return 2;}}}",
        options: Default::default(),
    });
}

#[test]
fn table64_operations_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/table64_operations.wat");
    assert_differential(DifferentialCase {
        name: "table64 operations",
        wat,
        native_expression: "(()=>{var before=m.size(),direct=m.apply(0n,20);m.copy(0n,1n,1n);var copied=m.apply(0n,20);m.set(1n);var set=m.apply(1n,20);m.fill(0n,1n);var old=m.grow(1n),after=m.size();return [i64Pair(before),direct,copied,set,i64Pair(old),i64Pair(after)]})()",
        asm_expression: "(()=>{var before=m.size(),beforeHigh=m.getTempRet0(),direct=m.apply(0,0,20);m.copy(0,0,1,0,1,0);var copied=m.apply(0,0,20);m.set(1,0);var set=m.apply(1,0,20);m.fill(0,0,1,0);var old=m.grow(1,0),oldHigh=m.getTempRet0(),after=m.size(),afterHigh=m.getTempRet0();return [[before,beforeHigh],direct,copied,set,[old,oldHigh],[after,afterHigh]]})()",
        native_imports: "{}",
        asm_imports: "{}",
        options: Default::default(),
    });
}
