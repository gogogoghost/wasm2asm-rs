use std::process::Command;
use wasm2asm::{CompileOptions, Lowerings, compile};

fn run_with_options(wat_source: &str, invocation: &str, options: &CompileOptions) -> String {
    let wasm = wat::parse_str(wat_source).unwrap();
    let js = String::from_utf8(compile(&wasm, options).unwrap()).unwrap();
    let script = format!("{js};let x={invocation};process.stdout.write(String(x));");
    let output = Command::new("node").args(["-e", &script]).output().unwrap();
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        output.status.success(),
        "node failed: {stderr}\nsource:\n{js}",
    );
    assert!(
        !stderr.contains("Invalid asm.js"),
        "V8 rejected generated asm.js: {stderr}\nsource:\n{js}",
    );
    String::from_utf8(output.stdout).unwrap()
}

fn run(wat_source: &str, invocation: &str) -> String {
    run_with_options(wat_source, invocation, &CompileOptions::default())
}

#[test]
fn scalar_add_runs() {
    assert_eq!(
        run(
            "(module (func (export \"add\") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))",
            "instantiate({}).add(20,22)"
        ),
        "42"
    );
}

#[test]
fn memory_round_trip_runs() {
    let wat = r#"(module
      (memory (export "memory") 1 2)
      (func (export "store") (param i32 i32) local.get 0 local.get 1 i32.store)
      (func (export "load") (param i32) (result i32) local.get 0 i32.load))"#;
    assert_eq!(
        run(
            wat,
            "(()=>{let m=instantiate({});m.store(4,305419896);return m.load(4)})()"
        ),
        "305419896"
    );
}

#[test]
fn local_get_preserves_evaluation_order() {
    let wat = r#"(module
      (func (export "snapshot") (param i32) (result i32)
        local.get 0
        i32.const 7
        local.set 0))"#;
    assert_eq!(run(wat, "instantiate({}).snapshot(42)"), "42");
}

#[test]
fn acyclic_tail_call_needs_no_switch() {
    let wat = r#"(module
      (func $increment (param i32) (result i32) local.get 0 i32.const 1 i32.add)
      (func (export "call") (param i32) (result i32) local.get 0 return_call $increment))"#;
    assert_eq!(run(wat, "instantiate({}).call(41)"), "42");
}

#[test]
fn recursive_tail_call_is_rejected_for_strict_asm_js() {
    let wat = r#"(module
      (func $sum (export "sum") (param $n i32) (param $acc i32) (result i32)
        local.get $n
        if (result i32)
          local.get $n
          i32.const 1
          i32.sub
          local.get $acc
          local.get $n
          i32.add
          return_call $sum
        else
          local.get $acc
        end))"#;
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
    let wat = r#"(module
      (func (export "lane") (param i32) (result i32)
        local.get 0
        i32x4.splat
        v128.const i32x4 1 2 3 4
        i32x4.add
        i32x4.extract_lane 2))"#;
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
    let wat = r#"(module
      (type $unary (func (param i32) (result i32)))
      (func $inc (type $unary) (param i32) (result i32)
        local.get 0 i32.const 1 i32.add)
      (elem declare func $inc)
      (func (export "apply") (param i32) (result i32)
        local.get 0 ref.func $inc call_ref $unary))"#;
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
    let wat = r#"(module
      (memory i64 1 2)
      (func (export "store") (param i64 i32) local.get 0 local.get 1 i32.store)
      (func (export "load") (param i64) (result i32) local.get 0 i32.load)
      (func (export "size") (result i64) memory.size))"#;
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
    let wat = r#"(module
      (type $payload (func (param i32)))
      (tag $error (type $payload))
      (func (export "throwing") (param i32)
        local.get 0 throw $error))"#;
    let wasm = wat::parse_str(wat).unwrap();
    let error = compile(&wasm, &CompileOptions::default()).unwrap_err();
    assert!(error.to_string().contains("exception handling"));
}

#[test]
fn mutable_i64_global_exposes_low_high_facade() {
    let wat = r#"(module
      (global $g (export "g") (mut i64) (i64.const 4294967298))
      (func (export "low") (result i32) global.get $g i32.wrap_i64))"#;
    assert_eq!(
        run(
            wat,
            "(()=>{let m=instantiate({});let a=[m.g.low,m.g.high,m.low()];m.g.low=7;m.g.high=-2;return a.concat([m.g.low,m.g.high,m.low()])})()"
        ),
        "2,1,2,7,-2,7"
    );
}
