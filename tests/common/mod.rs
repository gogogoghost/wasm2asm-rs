#![allow(dead_code)]

use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output};
use std::sync::LazyLock;

use tempfile::NamedTempFile;
use wasm2asm::{CompileOptions, OutputFormat, compile};

static SPIDERMONKEY: LazyLock<PathBuf> = LazyLock::new(|| {
    let executable = env::var_os("SPIDERMONKEY_JS")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("js140"));
    let output = Command::new(&executable)
        .arg("--version")
        .output()
        .unwrap_or_else(|error| {
            panic!(
                "cannot execute SpiderMonkey shell {}: {error}; install js140 or set SPIDERMONKEY_JS",
                executable.display()
            )
        });
    assert!(
        output.status.success(),
        "SpiderMonkey shell {} failed version check: {}",
        executable.display(),
        String::from_utf8_lossy(&output.stderr)
    );
    executable
});

fn spider_monkey() -> &'static PathBuf {
    &SPIDERMONKEY
}

fn run_script(name: &str, source: &str) -> Output {
    let mut script = NamedTempFile::with_suffix(".js").expect("create SpiderMonkey test script");
    script
        .write_all(source.as_bytes())
        .expect("write SpiderMonkey test script");
    let output = Command::new(spider_monkey())
        .args(["-e", "options('throw_on_asmjs_validation_failure')"])
        .arg(script.path())
        .output()
        .expect("run SpiderMonkey test script");
    if !output.status.success() {
        let (_, path) = script.keep().expect("preserve failing SpiderMonkey script");
        panic!(
            "SpiderMonkey case {name} failed:\n{}\nscript: {}",
            String::from_utf8_lossy(&output.stderr),
            path.display()
        );
    }
    output
}

fn compile_bare(wat_source: &str, options: &CompileOptions) -> (Vec<u8>, String) {
    let wasm = wat::parse_str(wat_source).expect("parse WAT fixture");
    let mut options = options.clone();
    options.output_format = OutputFormat::Bare;
    let javascript = String::from_utf8(compile(&wasm, &options).expect("compile WAT fixture"))
        .expect("compiler emitted UTF-8");
    (wasm, javascript)
}

fn strict_validation() -> &'static str {
    r#"
if (typeof isAsmJSCompilationAvailable !== "function" ||
    typeof isAsmJSModule !== "function") {
  throw new Error("SpiderMonkey shell does not expose asm.js validation APIs");
}
if (!isAsmJSCompilationAvailable()) {
  throw new Error("SpiderMonkey asm.js compilation is disabled");
}
if (!isAsmJSModule(asmModule)) {
  throw new Error("generated asmModule did not pass SpiderMonkey asm.js validation");
}
"#
}

pub fn run_asm_js(wat_source: &str, invocation: &str, options: &CompileOptions) -> String {
    let (_, javascript) = compile_bare(wat_source, options);
    let script = format!(
        "{javascript}\n{}\nvar value=({invocation});print(String(value));",
        strict_validation()
    );
    let output = run_script("asm.js execution", &script);
    String::from_utf8(output.stdout)
        .expect("SpiderMonkey stdout is UTF-8")
        .trim_end_matches(['\r', '\n'])
        .to_owned()
}

pub struct DifferentialCase<'a> {
    pub name: &'a str,
    pub wat: &'a str,
    pub native_expression: &'a str,
    pub asm_expression: &'a str,
    pub native_imports: &'a str,
    pub asm_imports: &'a str,
    pub options: CompileOptions,
}

impl<'a> DifferentialCase<'a> {
    pub fn same(name: &'a str, wat: &'a str, expression: &'a str) -> Self {
        Self {
            name,
            wat,
            native_expression: expression,
            asm_expression: expression,
            native_imports: "{}",
            asm_imports: "{}",
            options: CompileOptions::default(),
        }
    }
}

pub fn assert_differential(case: DifferentialCase<'_>) {
    let (wasm, javascript) = compile_bare(case.wat, &case.options);
    let wasm_bytes = wasm.iter().map(u8::to_string).collect::<Vec<_>>().join(",");
    let name = format!("{:?}", case.name);
    let script = format!(
        r#"
{javascript}
{validation}
function normalize(value) {{
  if (typeof value === "number") {{
    if (Number.isNaN(value)) return {{number:"NaN"}};
    if (value === Infinity) return {{number:"Infinity"}};
    if (value === -Infinity) return {{number:"-Infinity"}};
    if (Object.is(value, -0)) return {{number:"-0"}};
    return value;
  }}
  if (typeof value === "bigint") return {{bigint:String(value)}};
  if (Array.isArray(value)) return value.map(normalize);
  if (value && typeof value === "object") {{
    var keys=Object.keys(value).sort(), result={{}};
    for (var index=0;index<keys.length;index++) result[keys[index]]=normalize(value[keys[index]]);
    return result;
  }}
  return value;
}}
function i64Pair(value) {{
  return [
    Number(BigInt.asIntN(32, value)),
    Number(BigInt.asIntN(32, value >> 32n))
  ];
}}
function trapped(callback) {{
  try {{ callback(); return false; }} catch (_) {{ return true; }}
}}
var wasmBytes=new Uint8Array([{wasm_bytes}]);
var nativeModule=new WebAssembly.Module(wasmBytes);
var nativeExports=new WebAssembly.Instance(nativeModule,({native_imports})).exports;
var asmExports=instantiate(({asm_imports}));
var nativeResult=normalize((function(m){{return ({native_expression});}})(nativeExports));
var asmResult=normalize((function(m){{return ({asm_expression});}})(asmExports));
var nativeJson=JSON.stringify(nativeResult);
var asmJson=JSON.stringify(asmResult);
if (nativeJson !== asmJson) {{
  throw new Error("differential mismatch in " + {name} + "\nnative: " + nativeJson + "\nasm.js: " + asmJson);
}}
print(nativeJson);
"#,
        validation = strict_validation(),
        native_imports = case.native_imports,
        asm_imports = case.asm_imports,
        native_expression = case.native_expression,
        asm_expression = case.asm_expression,
    );
    run_script(case.name, &script);
}
