use crate::common::{DifferentialCase, assert_differential};
#[test]
fn structured_control_flow_matches_native_wasm() {
    let wat = include_str!("../fixtures/differential/structured_control_flow.wat");
    assert_differential(DifferentialCase::same(
        "structured control flow",
        wat,
        "[m.call(41),m.sum(10),m.choose(0),m.choose(1),m.choose(9),m.branchValue(0),m.branchValue(1),m.conditional(0),m.conditional(1)]",
    ));
}
#[test]
fn wasm_traps_are_preserved_by_asm_js() {
    let wat = include_str!("../fixtures/differential/wasm_traps_are_preserved_by_asm_js.wat");
    assert_differential(DifferentialCase::same(
        "trap preservation",
        wat,
        "[trapped(function(){m.unreachable()}),trapped(function(){m.divideByZero()}),trapped(function(){m.overflow()}),trapped(function(){m.outOfBounds()}),trapped(function(){m.invalidConversion()})]",
    ));
}
