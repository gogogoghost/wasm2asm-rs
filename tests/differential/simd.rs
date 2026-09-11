use crate::common::{DifferentialCase, assert_differential};
use wasm2asm::{CompileOptions, Lowerings};
#[test]
fn simd_lowering_matches_native_wasm() {
    let wat = include_str!("../fixtures/differential/simd_lowering.wat");
    let options = CompileOptions {
        lowerings: Lowerings::SIMD,
        ..CompileOptions::default()
    };
    assert_differential(DifferentialCase {
        name: "simd lowering",
        wat,
        native_expression: "(()=>{var r=m.run();return [r,Array.from(new Uint8Array(m.memory.buffer,0,368))]})()",
        asm_expression: "(()=>{var r=m.run();return [r,Array.from(new Uint8Array(m.memory.buffer,0,368))]})()",
        native_imports: "{}",
        asm_imports: "{}",
        options,
    });
}
