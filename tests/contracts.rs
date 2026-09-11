use wasm2asm::{
    CompileError, CompileOptions, ErrorKind, Lowerings, OutputFormat, ResourceLimits, compile,
    inspect,
};

fn wasm(wat_source: &str) -> Vec<u8> {
    wat::parse_str(wat_source).expect("parse WAT fixture")
}

fn compile_wat(wat_source: &str, options: &CompileOptions) -> Result<Vec<u8>, CompileError> {
    compile(&wasm(wat_source), options)
}

fn assert_resource_limit(
    wat_source: &str,
    configure: impl FnOnce(&mut ResourceLimits),
    resource: &str,
) {
    let mut options = CompileOptions::default();
    configure(&mut options.limits);
    let error = compile_wat(wat_source, &options).expect_err("resource limit must reject module");
    assert_eq!(error.kind, ErrorKind::ResourceLimit);
    assert!(
        error.message.contains(resource),
        "expected {resource:?} in {}",
        error.message
    );
}

#[test]
fn public_diagnostics_preserve_kind_offset_and_message() {
    let plain = CompileError::new(ErrorKind::Internal, "plain failure");
    assert_eq!(plain.kind, ErrorKind::Internal);
    assert_eq!(plain.offset, None);
    assert_eq!(plain.to_string(), "plain failure");

    let located = CompileError::at(ErrorKind::InvalidInput, 17, "bad opcode");
    assert_eq!(located.kind, ErrorKind::InvalidInput);
    assert_eq!(located.offset, Some(17));
    assert_eq!(located.to_string(), "bad opcode");

    let unsupported = CompileError::unsupported("threads", Some(23), "shared state");
    assert_eq!(unsupported.kind, ErrorKind::UnsupportedFeature);
    assert_eq!(unsupported.offset, Some(23));
    assert!(unsupported.to_string().contains("threads"));
    assert!(unsupported.to_string().contains("shared state"));

    let limit = CompileError::limit("functions", 3, 2);
    assert_eq!(limit.kind, ErrorKind::ResourceLimit);
    assert!(
        limit
            .to_string()
            .contains("value is 3; configured limit is 2")
    );

    let malformed = compile(&[0, 1, 2, 3], &CompileOptions::default()).unwrap_err();
    assert_eq!(malformed.kind, ErrorKind::InvalidInput);
    assert!(malformed.offset.is_some());
    assert!(malformed.to_string().contains("invalid WebAssembly input"));
}

#[test]
fn lowering_names_and_feature_inspection_are_public_contracts() {
    assert_eq!(Lowerings::parse(&[], false).unwrap(), Lowerings::empty());
    assert_eq!(
        Lowerings::parse(&["simd".into(), "references".into()], false).unwrap(),
        Lowerings::ALL
    );
    assert_eq!(Lowerings::parse(&[], true).unwrap(), Lowerings::ALL);
    assert_eq!(
        Lowerings::parse(&["all".into()], false).unwrap(),
        Lowerings::ALL
    );
    assert!(
        Lowerings::parse(&["unknown".into()], false)
            .unwrap_err()
            .contains("expected simd, references, or all")
    );

    let features = inspect(
        &wasm(
            r#"(module
              (memory i64 1)
              (func (result i64) i64.const 0)
              (func (result v128) v128.const i32x4 0 0 0 0))"#,
        ),
        &CompileOptions::default(),
    )
    .unwrap();
    assert!(features.contains("i64: true"));
    assert!(features.contains("simd: true"));
    assert!(features.contains("memory64: true"));
}

#[test]
fn every_resource_budget_rejects_oversized_modules() {
    let bytes = wasm("(module)");
    let mut options = CompileOptions::default();
    options.limits.max_input_bytes = bytes.len() - 1;
    let error = compile(&bytes, &options).unwrap_err();
    assert_eq!(error.kind, ErrorKind::ResourceLimit);
    assert!(error.message.contains("input bytes"));

    assert_resource_limit(
        "(module (func) (func))",
        |l| l.max_functions = 1,
        "functions",
    );
    assert_resource_limit(
        "(module (memory 1) (func (export \"f\")))",
        |l| l.max_module_elements = 1,
        "module elements",
    );
    assert_resource_limit(
        "(module (func (local i32 i32)))",
        |l| l.max_function_ir = 1,
        "function locals",
    );
    assert_resource_limit(
        "(module (func nop nop))",
        |l| l.max_function_ir = 1,
        "function IR expressions",
    );
    assert_resource_limit(
        "(module (func nop))",
        |l| l.max_total_ir = 1,
        "total IR expressions",
    );
    assert_resource_limit(
        "(module (memory 1) (data (i32.const 0) \"ab\"))",
        |l| l.max_segment_bytes = 1,
        "data segment bytes",
    );
    assert_resource_limit(
        "(module (func $a) (func $b) (table 2 funcref) (elem (i32.const 0) func $a $b))",
        |l| l.max_element_items = 1,
        "element segment items",
    );
    assert_resource_limit(
        "(module (memory 2))",
        |l| l.max_memory_pages = 1,
        "initial memory pages",
    );
    assert_resource_limit(
        "(module (table 2 funcref))",
        |l| l.max_table_elements = 1,
        "initial table elements",
    );
    assert_resource_limit(
        "(module (func))",
        |l| l.max_js_bytes = 1,
        "generated JavaScript bytes",
    );
}

#[test]
fn unsupported_boundaries_fail_with_specific_diagnostics() {
    let cases = [
        (
            "(module (memory 1 1 shared))",
            Lowerings::empty(),
            "threads or shared memory",
        ),
        (
            "(module (table 1 funcref) (table 1 funcref))",
            Lowerings::empty(),
            "multiple tables",
        ),
        (
            "(module (table (export \"table\") 1 funcref))",
            Lowerings::empty(),
            "imported or exported tables",
        ),
        (
            "(module (func (result v128) v128.const i32x4 0 0 0 0))",
            Lowerings::empty(),
            "simd",
        ),
        (
            r#"(module
              (type $unary (func (param i32) (result i32)))
              (func $id (type $unary) (param i32) (result i32) local.get 0)
              (elem declare func $id)
              (func (param i32) (result i32) local.get 0 ref.func $id call_ref $unary))"#,
            Lowerings::empty(),
            "typed function references",
        ),
    ];

    for (wat_source, lowerings, feature) in cases {
        let options = CompileOptions {
            lowerings,
            ..CompileOptions::default()
        };
        let error = compile_wat(wat_source, &options).expect_err("feature must be rejected");
        assert_eq!(error.kind, ErrorKind::UnsupportedFeature);
        assert!(
            error.message.contains(feature),
            "expected {feature:?} in {}",
            error.message
        );
    }

    let imported_multivalue = r#"(module
      (import "env" "pair" (func $pair (result i32 i32)))
      (func (export "run") (result i32 i32) call $pair))"#;
    let error = compile_wat(imported_multivalue, &CompileOptions::default()).unwrap_err();
    assert_eq!(error.kind, ErrorKind::UnsupportedFeature);
    assert!(error.message.contains("imported multivalue function"));

    let exported_vector = r#"(module
      (func (export "vector") (result v128) v128.const i32x4 0 0 0 0))"#;
    let options = CompileOptions {
        lowerings: Lowerings::SIMD,
        ..CompileOptions::default()
    };
    let error = compile_wat(exported_vector, &options).unwrap_err();
    assert_eq!(error.kind, ErrorKind::UnsupportedFeature);
    assert!(error.message.contains("SIMD host boundary"));
}

#[test]
fn output_formats_honor_their_embedding_contracts() {
    let wat = wasm("(module (func (export \"answer\") (result i32) i32.const 42))");

    let bare = String::from_utf8(
        compile(
            &wat,
            &CompileOptions {
                output_format: OutputFormat::Bare,
                ..CompileOptions::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(bare.contains("function instantiate("));
    assert!(!bare.contains("export default"));

    let esm = String::from_utf8(compile(&wat, &CompileOptions::default()).unwrap()).unwrap();
    assert!(esm.ends_with("export default instantiate;\n"));

    let umd = String::from_utf8(
        compile(
            &wat,
            &CompileOptions {
                output_format: OutputFormat::Umd,
                global_name: "CustomModule".into(),
                ..CompileOptions::default()
            },
        )
        .unwrap(),
    )
    .unwrap();
    assert!(umd.contains("CustomModule"));

    let error = compile(
        &wat,
        &CompileOptions {
            output_format: OutputFormat::Umd,
            global_name: String::new(),
            ..CompileOptions::default()
        },
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidInput);
    assert!(error.message.contains("--global-name must not be empty"));
}
