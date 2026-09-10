use std::process::Command;
use wasm2asm::{CompileOptions, Lowerings, OutputFormat, compile};

fn run_with_options(wat_source: &str, invocation: &str, options: &CompileOptions) -> String {
    let wasm = wat::parse_str(wat_source).unwrap();
    let mut options = options.clone();
    options.output_format = OutputFormat::Bare;
    let js = String::from_utf8(compile(&wasm, &options).unwrap()).unwrap();
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
fn nested_float_additions_generate_valid_javascript() {
    let wat = r#"(module
      (func (export "sum") (result f64)
        f64.const 1
        f64.const 2
        f64.add
        f64.const 3
        f64.const 4
        f64.add
        f64.add))"#;
    assert_eq!(run(wat, "instantiate({}).sum()"), "10");
}

#[test]
fn i64_select_runs_in_strict_module() {
    let wat = r#"(module
      (func (export "pick") (param i32 i64 i64) (result i64)
        local.get 1
        local.get 2
        local.get 0
        select))"#;
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
    let wat = r#"(module
      (func (export "lt") (param i64 i64) (result i32)
        local.get 0
        local.get 1
        i64.lt_s))"#;
    assert_eq!(
        run(
            wat,
            "(()=>{let m=instantiate({});return [m.lt(-1,-1,0,0),m.lt(0,0,-1,-1),m.lt(0,0,1,0)]})()"
        ),
        "1,0,1"
    );
}

#[test]
fn pooled_temporaries_preserve_loop_state() {
    let wat = r#"(module
      (func (export "sum") (param $n i32) (result i32)
        (local $i i32) (local $acc i32)
        local.get $n
        local.set $i
        i32.const 0
        local.set $acc
        block $exit
          loop $loop
            local.get $i
            i32.eqz
            br_if $exit
            local.get $acc
            local.get $i
            i32.const 3
            i32.mul
            i32.add
            local.set $acc
            local.get $i
            i32.const 1
            i32.sub
            local.set $i
            br $loop
          end
        end
        local.get $acc))"#;
    assert_eq!(run(wat, "instantiate({}).sum(10)"), "165");
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

#[test]
fn default_es_module_output_imports_directly() {
    let wasm = wat::parse_str(
        "(module (func (export \"add\") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))",
    )
    .unwrap();
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
    let wasm = wat::parse_str(
        "(module (func (export \"add\") (param i32 i32) (result i32) local.get 0 local.get 1 i32.add))",
    )
    .unwrap();
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
