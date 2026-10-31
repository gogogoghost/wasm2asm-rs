use crate::common::{DifferentialCase, assert_differential};
use wasm2asm::CompileOptions;

#[test]
fn f64_local_control_flow_preserves_parameter_and_local_bits() {
    let wat = include_str!("../fixtures/differential/f64_local_control_flow.wat");
    let expression = r#"(()=>{
        var values=[0,-0,1,-1,900,8000,900.25,Infinity,-Infinity];
        var bits=[0n,0x8000000000000000n,0x7ff0000000000000n,
            0xfff0000000000000n,0x7ff0000000000001n,0x7ff8123456789abcn,
            0xfff123456789abcdn,0x0123456789abcdefn];
        var result=[];
        for (var a of values) for (var b of values) {
            for (var take of [0,1]) for (var name of ['choose','store_choice','merge'])
                result.push(PAIR(m[name](a,b,take)));
            for (var n of [1,2,3,4]) result.push(PAIR(m.loop_param(a,b,n)));
        }
        for (var value of bits) {
            for (var n of [1,2,3,4]) result.push(PAIR(m.loop_local(BITS,n)));
            for (var take of [0,1]) result.push(PAIR(m.branch_local(BITS,take)));
        }
        return result;
    })()"#;
    let native_expression = expression
        .replace("PAIR(", "i64Pair(")
        .replace("BITS", "value");
    let asm_expression = expression
        .replace("PAIR(", "pair(")
        .replace("BITS", "...i64Pair(value)")
        .replace(
            "var result=[];",
            "function pair(low){return [low,m.getTempRet0()]} var result=[];",
        );
    for (preserve_traps, assume_memory_alignment) in [(true, false), (false, false), (false, true)]
    {
        assert_differential(DifferentialCase {
            name: "f64 local control flow",
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

#[test]
fn low_word_left_shifts_keep_count_masking_effects_and_load_traps() {
    let wat = include_str!("../fixtures/differential/i64_low_word_shifts.wat");
    let expression = r#"(()=>{
        var values = [0n,1n,-1n,0x7fffffffn,0x80000000n,0xffffffffn,
            0x100000000n,0x8123456789abcdefn];
        var counts = [0n,1n,31n,32n,33n,63n,64n,65n,127n,-1n,-32n,
            0x100000001n,0x8000000000000020n];
        var result = values.map(value => ({
            constant: CONSTANT,
            dynamic: counts.map(count => DYNAMIC)
        }));
        result.push(counts.map(count => EFFECTFUL));
        result.push(counts.map(count => LOAD));
        if (STRICT) {
            var trapped = false;
            try { INVALID_LOAD; } catch (error) { trapped = true; }
            result.push(trapped);
        }
        return result;
    })()"#;
    let native_expression = expression
        .replace("CONSTANT", "m.constant(value)")
        .replace("DYNAMIC", "m.dynamic(value,count)")
        .replace("EFFECTFUL", "m.effectful(count)")
        .replace("INVALID_LOAD", "m.load_shift(65530,32n)")
        .replace("LOAD", "m.load_shift(65528,count)");
    let asm_expression = expression
        .replace("CONSTANT", "m.constant(...i64Pair(value))")
        .replace("DYNAMIC", "m.dynamic(...i64Pair(value),...i64Pair(count))")
        .replace("EFFECTFUL", "m.effectful(...i64Pair(count))")
        .replace("INVALID_LOAD", "m.load_shift(65530,...i64Pair(32n))")
        .replace("LOAD", "m.load_shift(65528,...i64Pair(count))");
    for preserve_traps in [true, false] {
        let strict = if preserve_traps { "true" } else { "false" };
        assert_differential(DifferentialCase {
            name: "low-word left shifts",
            wat,
            native_expression: &native_expression.replace("STRICT", strict),
            asm_expression: &asm_expression.replace("STRICT", strict),
            native_imports: "{}",
            asm_imports: "{}",
            options: CompileOptions {
                preserve_traps,
                ..CompileOptions::default()
            },
        });
    }
}
