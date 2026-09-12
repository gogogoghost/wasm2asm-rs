mod common;

use std::process::Command;
use wasm2asm::{CompileOptions, Lowerings, OutputFormat, compile};

fn run_with_options(wat_source: &str, invocation: &str, options: &CompileOptions) -> String {
    common::run_asm_js(wat_source, invocation, options)
}

fn run(wat_source: &str, invocation: &str) -> String {
    run_with_options(wat_source, invocation, &CompileOptions::default())
}

#[test]
fn scalar_add_runs() {
    let wat = include_str!("fixtures/smoke/scalar_add_runs.wat");
    assert_eq!(run(wat, "instantiate({}).add(20,22)"), "42");
}

#[test]
fn nested_float_additions_generate_valid_javascript() {
    let wat = include_str!("fixtures/smoke/nested_float_additions_generate_valid_javascript.wat");
    assert_eq!(run(wat, "instantiate({}).sum()"), "10");
}

#[test]
fn i64_select_runs_in_strict_module() {
    let wat = include_str!("fixtures/smoke/i64_select_runs_in_strict_module.wat");
    assert_eq!(
        run(
            wat,
            "(()=>{let m=instantiate({});let low=m.pick(1,11,0,22,0);return [low,m.getTempRet0()]})()"
        ),
        "11,0"
    );
}

#[test]
fn i64_comparison_runs_in_strict_module() {
    let wat = include_str!("fixtures/smoke/i64_comparison_runs_in_strict_module.wat");
    assert_eq!(
        run(
            wat,
            "(()=>{let m=instantiate({});return [m.lt(-1,-1,0,0),m.lt(0,0,-1,-1),m.lt(0,0,1,0)]})()"
        ),
        "1,0,1"
    );
}

#[test]
fn i64_comparison_controls_strict_branch() {
    let wat = include_str!("fixtures/smoke/i64_comparison_controls_strict_branch.wat");
    assert_eq!(
        run(
            wat,
            "(()=>{let m=instantiate({});return [m.equal(1,0,1,0),m.equal(1,0,2,0)]})()"
        ),
        "1,0"
    );
}

#[test]
fn i64_bit_counts_run_in_strict_module() {
    let wat = include_str!("fixtures/smoke/i64_bit_counts_run_in_strict_module.wat");
    assert_eq!(
        run(
            wat,
            "(()=>{let m=instantiate({});return [m.clz(0,0),m.clz(0,1),m.ctz(0,1),m.ctz(8,0)]})()"
        ),
        "64,31,32,3"
    );
}

#[test]
fn pooled_temporaries_preserve_loop_state() {
    let wat = include_str!("fixtures/smoke/pooled_temporaries_preserve_loop_state.wat");
    assert_eq!(run(wat, "instantiate({}).sum(10)"), "165");
}

#[test]
fn memory_round_trip_runs() {
    let wat = include_str!("fixtures/smoke/memory_round_trip_runs.wat");
    assert_eq!(
        run(
            wat,
            "(()=>{let m=instantiate({});m.store(4,305419896);return m.load(4)})()"
        ),
        "305419896"
    );
}

#[test]
fn memory_offset_overflow_traps() {
    let wat = include_str!("fixtures/smoke/memory_offset_overflow_traps.wat");
    assert_eq!(
        run(
            wat,
            "(()=>{let m=instantiate({});let trapped=false;try{m.load_offset(-4)}catch{trapped=true}return [m.load_offset(4),trapped]})()"
        ),
        "0,true"
    );
}

#[test]
fn local_get_preserves_evaluation_order() {
    let wat = include_str!("fixtures/smoke/local_get_preserves_evaluation_order.wat");
    assert_eq!(run(wat, "instantiate({}).snapshot(42)"), "42");
}

#[test]
fn local_tee_preserves_popped_temporary() {
    let wat = include_str!("fixtures/smoke/local_tee_preserves_popped_temporary.wat");
    assert_eq!(run(wat, "instantiate({}).snapshot()"), "42");
}

#[test]
fn acyclic_tail_call_needs_no_switch() {
    let wat = include_str!("fixtures/smoke/acyclic_tail_call_needs_no_switch.wat");
    assert_eq!(run(wat, "instantiate({}).call(41)"), "42");
}

#[test]
fn recursive_tail_call_is_rejected_for_strict_asm_js() {
    let wat = include_str!("fixtures/smoke/recursive_tail_call_is_rejected_for_strict_asm_js.wat");
    let wasm = wat::parse_str(wat).unwrap();
    let error = compile(&wasm, &CompileOptions::default()).unwrap_err();
    assert!(
        error
            .to_string()
            .contains("recursive or indirect tail calls")
    );
}

#[test]
fn simd_lowering_runs_in_strict_module() {
    let wat = include_str!("fixtures/smoke/simd_lowering_runs_in_strict_module.wat");
    let options = CompileOptions {
        lowerings: Lowerings::SIMD,
        ..CompileOptions::default()
    };
    assert_eq!(
        run_with_options(wat, "instantiate({}).lane(39)", &options),
        "42"
    );
}

#[test]
fn typed_reference_lowering_runs_in_strict_module() {
    let wat = include_str!("fixtures/smoke/typed_reference_lowering_runs_in_strict_module.wat");
    let options = CompileOptions {
        lowerings: Lowerings::REFERENCES,
        ..CompileOptions::default()
    };
    assert_eq!(
        run_with_options(wat, "instantiate({}).apply(41)", &options),
        "42"
    );
}

#[test]
fn memory64_uses_low_high_i32_abi() {
    let wat = include_str!("fixtures/smoke/memory64_uses_low_high_i32_abi.wat");
    assert_eq!(
        run(
            wat,
            "(()=>{let m=instantiate({});m.store(8,0,42);return [m.load(8,0),m.size(),m.getTempRet0()]})()"
        ),
        "42,1,0"
    );
}

#[test]
fn exceptions_are_rejected_for_strict_asm_js() {
    let wat = include_str!("fixtures/smoke/exceptions_are_rejected_for_strict_asm_js.wat");
    let wasm = wat::parse_str(wat).unwrap();
    let error = compile(&wasm, &CompileOptions::default()).unwrap_err();
    assert!(error.to_string().contains("exception handling"));
}

#[test]
fn mutable_i64_global_exposes_low_high_facade() {
    let wat = include_str!("fixtures/smoke/mutable_i64_global_exposes_low_high_facade.wat");
    assert_eq!(
        run(
            wat,
            "(()=>{let m=instantiate({});let a=[m.g.low,m.g.high,m.low()];m.g.low=7;m.g.high=-2;return a.concat([m.g.low,m.g.high,m.low()])})()"
        ),
        "2,1,2,7,-2,7"
    );
}

#[test]
fn default_es_module_output_imports_directly() {
    let wasm = wat::parse_str(include_str!("fixtures/smoke/scalar_add_runs.wat")).unwrap();
    let js = String::from_utf8(compile(&wasm, &CompileOptions::default()).unwrap()).unwrap();
    assert!(js.contains("export default instantiate"));
    let script = format!("{js}\nlet m=instantiate({{}});if(m.add(20,22)!==42)throw Error('esm');");
    let output = Command::new("node")
        .args(["--input-type=module", "-e", &script])
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "ES module failed: {stderr}");
    assert!(
        !stderr.contains("Invalid asm.js"),
        "V8 rejected ES module core: {stderr}"
    );
}

#[test]
fn umd_output_supports_browser_global_and_commonjs() {
    let wasm = wat::parse_str(include_str!("fixtures/smoke/scalar_add_runs.wat")).unwrap();
    let options = CompileOptions {
        output_format: OutputFormat::Umd,
        global_name: "LegacyAdder".into(),
        ..CompileOptions::default()
    };
    let js = String::from_utf8(compile(&wasm, &options).unwrap()).unwrap();
    let script = format!(
        "var source={};var self={{}};new Function('module','self',source)(undefined,self);var browser=self.LegacyAdder.instantiate({{}});if(browser.add(20,22)!==42)throw Error('umd global');var module={{exports:{{}}}};new Function('module','self',source)(module,{{}});var commonjs=module.exports.instantiate({{}});if(commonjs.add(19,23)!==42)throw Error('umd commonjs');",
        js_string_for_test(&js)
    );
    let output = Command::new("node").args(["-e", &script]).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(output.status.success(), "UMD output failed: {stderr}");
    assert!(
        !stderr.contains("Invalid asm.js"),
        "V8 rejected UMD core: {stderr}"
    );
}

fn js_string_for_test(value: &str) -> String {
    let mut out = String::from("\"");
    for byte in value.bytes() {
        match byte {
            b'\\' => out.push_str("\\\\"),
            b'\"' => out.push_str("\\\""),
            b'\n' => out.push_str("\\n"),
            b'\r' => out.push_str("\\r"),
            byte => out.push(byte as char),
        }
    }
    out.push('\"');
    out
}
