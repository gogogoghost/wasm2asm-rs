use crate::common::{DifferentialCase, assert_differential};
use wasm2asm::CompileOptions;

#[test]
fn i32_numeric_operations_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/i32_numeric_operations.wat");
    assert_differential(DifferentialCase::same(
        "i32 numeric operations",
        wat,
        "m.run()",
    ));
}

#[test]
fn fast_numeric_operations_match_native_wasm_for_valid_inputs() {
    let wat = include_str!("../fixtures/differential/i32_numeric_operations.wat");
    let options = CompileOptions {
        preserve_traps: false,
        ..CompileOptions::default()
    };
    assert_differential(DifferentialCase {
        name: "fast i32 numeric operations",
        wat,
        native_expression: "m.run()",
        asm_expression: "m.run()",
        native_imports: "{}",
        asm_imports: "{}",
        options,
    });
}

#[test]
fn fast_numeric_boundaries_match_native_wasm_for_valid_inputs() {
    let wat = include_str!("../fixtures/differential/fast_valid_numeric_boundaries.wat");
    let options = CompileOptions {
        preserve_traps: false,
        ..CompileOptions::default()
    };
    assert_differential(DifferentialCase {
        name: "fast numeric boundaries",
        wat,
        native_expression: "m.run()",
        asm_expression: "m.run()",
        native_imports: "{}",
        asm_imports: "{}",
        options,
    });
}
#[test]
fn i64_numeric_operations_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/i64_numeric_operations.wat");
    assert_differential(DifferentialCase {
        name: "i64 numeric operations",
        wat,
        native_expression: "({ops:m.ops().map(i64Pair),cmp:m.cmp()})",
        asm_expression: "({ops:m.ops(),cmp:m.cmp()})",
        native_imports: "{}",
        asm_imports: "{}",
        options: Default::default(),
    });
}
#[test]
fn floating_point_operations_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/floating_point_operations.wat");
    assert_differential(DifferentialCase::same(
        "floating point operations",
        wat,
        "({f32:m.f32ops(),f64:m.f64ops(),cmp:m.cmp()})",
    ));
}
#[test]
fn numeric_conversions_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/numeric_conversions.wat");
    assert_differential(DifferentialCase {
        name: "numeric conversions",
        wat,
        native_expression: "(()=>{var r=m.reinterpret(),e=m.extend();return {i32:m.i32trunc(),i64:m.i64trunc().map(i64Pair),f32:m.toF32(),f64:m.toF64(),reinterpret:[r[0],i64Pair(r[1]),r[2],r[3]],extend:[e[0],e[1],i64Pair(e[2]),i64Pair(e[3]),i64Pair(e[4])]}})()",
        asm_expression: "({i32:m.i32trunc(),i64:m.i64trunc(),f32:m.toF32(),f64:m.toF64(),reinterpret:m.reinterpret(),extend:m.extend()})",
        native_imports: "{}",
        asm_imports: "{}",
        options: Default::default(),
    });
}

#[test]
fn i64_rotation_boundaries_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/i64_rotation_boundaries.wat");
    // Different halves expose the rotate-by-32 bug; include signed halves and
    // counts outside 0..64 to check Wasm's masking of the shift count.
    let expression = r#"(()=>{
        var values = [0n, 1n, -1n, 0x0123456789abcdefn,
            0x76543210fedcba98n, 0xffffffff00000000n,
            0x00000000ffffffffn, 0x8000000000000001n];
        var counts = Array.from({length:128}, (_, i) => BigInt(i))
            .concat([-1n, -32n, -64n, -96n, 0x100000020n]);
        return values.map(value => ({
            constant: CONSTANT_CALL,
            dynamic: counts.map(count => DYNAMIC_CALL)
        }));
    })()"#;
    let native_expression = expression
        .replace("CONSTANT_CALL", "m.constant_counts(value).map(i64Pair)")
        .replace("DYNAMIC_CALL", "m.ops(value, count).map(i64Pair)");
    let asm_expression = expression
        .replace("CONSTANT_CALL", "m.constant_counts(...i64Pair(value))")
        .replace(
            "DYNAMIC_CALL",
            "m.ops(...i64Pair(value), ...i64Pair(count))",
        );
    for preserve_traps in [true, false] {
        assert_differential(DifferentialCase {
            name: "i64 rotation boundaries",
            wat,
            native_expression: &native_expression,
            asm_expression: &asm_expression,
            native_imports: "{}",
            asm_imports: "{}",
            options: CompileOptions {
                preserve_traps,
                ..CompileOptions::default()
            },
        });
    }
}
