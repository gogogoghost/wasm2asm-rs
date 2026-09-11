use super::*;
use std::panic;

fn function_type(params: Vec<ValType>, results: Vec<ValType>) -> FuncType {
    FuncType { params, results }
}

fn export(kind: ExportKind, index: u32) -> Export {
    Export {
        name: "value".into(),
        kind,
        index,
    }
}

fn import(kind: ImportKind) -> Import {
    Import {
        module: "env".into(),
        name: "value".into(),
        kind,
    }
}

#[test]
fn host_boundary_validation_covers_every_rejected_shape() {
    for kind in [ExportKind::Table, ExportKind::Tag] {
        let mut module = Module::default();
        module.exports.push(export(kind, 0));
        assert!(validate_boundaries(&module).is_err());
    }

    let mut invalid_function = Module::default();
    invalid_function.exports.push(export(ExportKind::Func, 0));
    assert!(validate_boundaries(&invalid_function).is_err());

    for ty in [ValType::V128, ValType::FuncRef(None)] {
        let mut module = Module::default();
        module.types.push(function_type(vec![], vec![ty]));
        module.function_type_indices.push(0);
        module.exports.push(export(ExportKind::Func, 0));
        assert!(validate_boundaries(&module).is_err());
    }

    let mut invalid_global = Module::default();
    invalid_global.exports.push(export(ExportKind::Global, 0));
    assert!(validate_boundaries(&invalid_global).is_err());

    for ty in [ValType::V128, ValType::FuncRef(None)] {
        let mut module = Module::default();
        module.global_types.push(GlobalType { ty, mutable: false });
        module.exports.push(export(ExportKind::Global, 0));
        assert!(validate_boundaries(&module).is_err());
    }

    let mut multivalue = Module::default();
    multivalue
        .types
        .push(function_type(vec![], vec![ValType::I32, ValType::I32]));
    multivalue.imports.push(import(ImportKind::Func(0)));
    assert!(validate_boundaries(&multivalue).is_err());

    for ty in [ValType::V128, ValType::FuncRef(None)] {
        let mut module = Module::default();
        module.types.push(function_type(vec![ty], vec![]));
        module.imports.push(import(ImportKind::Func(0)));
        assert!(validate_boundaries(&module).is_err());

        let mut module = Module::default();
        module.imports.push(import(ImportKind::Global(GlobalType {
            ty,
            mutable: false,
        })));
        assert!(validate_boundaries(&module).is_err());
    }
}

#[test]
fn value_and_javascript_helpers_cover_every_shape() {
    let values = [
        Value::I32("i".into()),
        Value::I64("lo".into(), "hi".into()),
        Value::F32("f".into()),
        Value::F64("d".into()),
        Value::Ref("r".into()),
        Value::V128(["a".into(), "b".into(), "c".into(), "d".into()]),
    ];
    assert_eq!(values[0].ty(), ValType::I32);
    assert_eq!(values[1].ty(), ValType::I64);
    assert_eq!(values[2].ty(), ValType::F32);
    assert_eq!(values[3].ty(), ValType::F64);
    assert_eq!(values[4].ty(), ValType::FuncRef(None));
    assert_eq!(values[5].ty(), ValType::V128);
    assert_eq!(values[1].components(), vec!["lo", "hi"]);
    assert_eq!(values[5].components(), vec!["a", "b", "c", "d"]);
    assert_eq!(values[0].i32_expr().unwrap(), "i");
    assert_eq!(values[4].i32_expr().unwrap(), "r");
    assert!(values[2].i32_expr().is_err());
    assert!(values[0].mentions_identifier("i"));
    assert!(!values[0].mentions_identifier("x"));
    assert!(Value::I32("x+y".into()).mentions_identifier("x"));
    assert!(!Value::I32("xx+y".into()).mentions_identifier("x"));
    assert!(!Value::I32("y+xx".into()).mentions_identifier("x"));

    assert!(type_compatible(
        ValType::FuncRef(None),
        ValType::FuncRef(Some(1))
    ));
    assert!(!type_compatible(ValType::I32, ValType::F32));
    for ty in [
        ValType::I32,
        ValType::I64,
        ValType::F32,
        ValType::F64,
        ValType::FuncRef(Some(0)),
        ValType::V128,
    ] {
        let value = named_value(ty, "n");
        assert!(type_compatible(value.ty(), ty));
    }
    assert_eq!(component_type(&values, "hi"), Some(ValType::I32));
    assert_eq!(component_type(&values, "missing"), None);
    assert_eq!(parameter_values(&[ValType::I64, ValType::F32]).len(), 2);
    assert_eq!(flatten_names(&values).len(), 10);
    assert_eq!(flatten_call_arguments(&values[..4], false).len(), 5);
    let external = flatten_call_arguments(&values[..4], true);
    assert!(external.iter().any(|value| value.starts_with("+(")));
    assert_eq!(zero_literal(ValType::F32), "F(0)");
    assert_eq!(zero_literal(ValType::F64), "0.0");
    assert_eq!(zero_literal(ValType::I64), "0");
    assert_eq!(coerce(ValType::I32, "x|0"), "x|0");
    assert_eq!(coerce(ValType::I32, "x"), "x|0");
    assert_eq!(coerce(ValType::F32, "F(x)"), "F(x)");
    assert_eq!(coerce(ValType::F32, "x"), "F(x)");
    assert_eq!(coerce(ValType::F64, "+x"), "+x");
    assert_eq!(coerce(ValType::F64, "x"), "+x");

    let mut out = String::new();
    emit_param_coercions(&mut out, &values);
    emit_var_value(&mut out, &values[1], "source");
    emit_var_value(&mut out, &values[5], "source");
    emit_var_value(&mut out, &values[2], "source");
    emit_var_value_from_value(&mut out, &values[1], &values[1]);
    for value in &values {
        emit_return_value(&mut out, value);
        emit_set_from_single_argument(&mut out, value, "arg");
    }
    emit_direct_js_return(&mut out, &[], "f()", false);
    emit_direct_js_return(&mut out, &[ValType::F32], "f()", true);
    emit_direct_js_return(&mut out, &[ValType::F32], "f()", false);
    emit_direct_js_return(&mut out, &[ValType::F64], "f()", false);
    emit_direct_js_return(&mut out, &[ValType::I32], "f()", false);
    emit_primary_export_value(&mut out, Some(ValType::I64), "x", Some(3));
    emit_primary_export_value(&mut out, Some(ValType::I32), "x", None);
    assert!(out.contains("source.low"));
}

#[test]
fn literal_and_identifier_helpers_cover_all_lexical_cases() {
    assert_eq!(
        compiled_function_body("function A(){return 1;}"),
        Some("return 1;")
    );
    assert_eq!(compiled_function_body("no braces"), None);
    assert_eq!(compiled_function_body("}"), None);
    assert_eq!(compiled_function_body("}{"), None);
    assert_eq!(call_name_before("env.f   (1)", 8), Some("env.f"));
    assert_eq!(call_name_before(" (1)", 1), None);
    assert!(literal_requires_constant_call_argument(Some("l2"), 2));
    assert!(literal_requires_constant_call_argument(Some("sf"), 1));
    assert!(!literal_requires_constant_call_argument(Some("f"), 0));

    let body = r#"f(12,"34\"56",'78',`90`);l(x,2);z=1.25;u=a1+5"#;
    let literals = numeric_literals(body);
    assert!(
        numeric_literals("switch(x){case 3:return 4}")
            .iter()
            .any(|literal| literal.case_label)
    );
    assert!(literals.iter().any(|literal| literal.must_literal));
    let key = numeric_template_key(body, &literals);
    let rendered = render_numeric_template_body(body, &literals, &[0]);
    assert!(!key.is_empty() && rendered.contains("c0"));

    assert!(!is_js_identifier(""));
    assert!(is_js_identifier("$valid_2"));
    assert!(!is_js_identifier("2bad"));
    assert!(is_js_callee("env.call"));
    assert!(!is_js_callee("env..call"));
    assert!(!is_js_call("name"));
    assert!(!is_js_call("bad-name()"));
    assert!(is_js_call("f((1))"));
    assert!(!is_js_call("f()x"));
    assert!(!is_js_call("f(()"));
    assert!(!is_fully_parenthesized("x"));
    assert!(is_fully_parenthesized("((x))"));
    assert!(!is_fully_parenthesized("(x)+(y)"));
    assert!(!is_fully_parenthesized("((x)"));
    assert!(is_js_atom("-1.5e2"));
    assert!(!is_js_atom("x+y"));
    assert_eq!(compact_operand("-x"), "(-x)");
    assert_eq!(compact_operand("x"), "x");
    assert_eq!(compact_operand("x+y"), "(x+y)");
    assert_eq!(compact_i32("x|0"), "x|0");
    assert_eq!(compact_i32("x"), "x|0");
    assert_eq!(compact_i32("x+y"), "(x+y)|0");
    assert_eq!(compact_float_argument("+x"), "+x");
    assert_eq!(compact_float_argument("x"), "+x");
    assert_eq!(compact_float_argument("x+y"), "+(x+y)");
    assert_eq!(compact_condition("((x|0)==0)|0"), "!x");
    assert_eq!(compact_condition("((x|0)!=0)|0"), "x");
    assert_eq!(compact_condition("((x|0)<(16|0))|0"), "(x|0)<16");
    assert_eq!(compact_condition("(x+y)|0"), "(x+y)|0");
    assert_eq!(js_ident(26, false), "aa");
    assert_eq!(js_ident(26, true), "AA");
    assert_eq!(short_index(26), "aa");
    assert!(!compact_index(20).is_empty());
    assert_eq!(
        js_string("\"\\\n\r\u{2028}\u{2029}\u{0001}"),
        "\"\\\"\\\\\\n\\r\\u2028\\u2029\\u0001\""
    );
    assert_eq!(
        js_byte_string(&[0x41, 0x42, 0xd8, 0, 0, 0x22]),
        "\"䅂\\ud800\\\"\""
    );
    assert_eq!(instruction_offset("a", "b"), 0);
}

#[test]
fn repeated_static_memory_accesses_are_extracted_without_changing_arguments() {
    let repeated = "a=l(100|0,0)|0;st(104|0,0,a,0);".repeat(20);
    let mut compiled = [CompiledFunction {
        index: 0,
        code: format!("function A(){{var a=0;{repeated}return a|0}}"),
    }];

    let helpers = optimize_static_memory_accesses(&mut compiled);

    assert!(helpers.contains("return l(100|0,0)|0"));
    assert!(helpers.contains("st(104|0,0|0,a|0,0|0)"));
    assert!(!compiled[0].code.contains("l(100|0,0)"));
    assert!(!compiled[0].code.contains("st(104|0,0,a,0)"));
    assert_eq!(compiled[0].code.matches("(a);").count(), 20);
}

#[test]
fn constants_and_return_abi_cover_every_value_kind() {
    let globals = [Value::I32("g".into())];
    for expr in [
        ConstExpr::I32(-1),
        ConstExpr::I64(-2),
        ConstExpr::F32(1.5f32.to_bits()),
        ConstExpr::F64(2.5f64.to_bits()),
        ConstExpr::GlobalGet(0),
        ConstExpr::RefNull,
        ConstExpr::RefFunc(3),
    ] {
        assert!(const_value(&expr, &globals).is_ok());
    }
    assert!(const_value(&ConstExpr::GlobalGet(1), &globals).is_err());
    assert_eq!(float32_literal(f32::NAN.to_bits()), "F(Na)");
    assert_eq!(float32_literal(f32::INFINITY.to_bits()), "F(In)");
    assert_eq!(float32_literal(f32::NEG_INFINITY.to_bits()), "F(-In)");
    assert_eq!(float32_literal((-0.0f32).to_bits()), "F(-0.0)");
    assert!(float32_literal(1.25f32.to_bits()).starts_with("F("));
    assert_eq!(float64_literal(f64::NAN.to_bits()), "Na");
    assert_eq!(float64_literal(f64::INFINITY.to_bits()), "In");
    assert_eq!(float64_literal(f64::NEG_INFINITY.to_bits()), "-In");
    assert_eq!(float64_literal((-0.0f64).to_bits()), "-0.0");
    assert_eq!(float64_literal(1.25f64.to_bits()), "1.25");

    let results = [
        ValType::V128,
        ValType::I64,
        ValType::V128,
        ValType::I32,
        ValType::FuncRef(None),
        ValType::F32,
        ValType::F64,
    ];
    let slot_types = return_slot_types(&results);
    let slots: Vec<Value> = slot_types
        .iter()
        .enumerate()
        .map(|(index, &ty)| named_value(ty, &format!("s{index}")))
        .collect();
    assert!(slot_types.len() > results.len());
    assert_eq!(return_slot_class(ValType::I32), 0);
    assert_eq!(return_slot_class(ValType::F32), 1);
    assert_eq!(return_slot_class(ValType::F64), 2);
    assert!(panic::catch_unwind(|| return_slot_class(ValType::I64)).is_err());

    let mut module = Module::default();
    module.types.push(function_type(vec![], results.to_vec()));
    let (counts, layout) = return_slot_layout(&module);
    assert_eq!(
        return_slot_indices(&results, counts).len(),
        slot_types.len()
    );
    assert_eq!(layout.len(), slot_types.len());

    let mut out = String::new();
    emit_return_slot_assignment(
        &mut out,
        &Value::F64("d".into()),
        &Value::I32("i".into()),
        "i",
    );
    emit_return_slot_assignment(
        &mut out,
        &Value::F64("d".into()),
        &Value::F32("f".into()),
        "f",
    );
    emit_return_slot_assignment(
        &mut out,
        &Value::I32("i".into()),
        &Value::I32("j".into()),
        "j",
    );
    emit_return_abi(&mut out, &[], &[]);
    emit_return_abi(
        &mut out,
        &[Value::V128([
            "a".into(),
            "b".into(),
            "c".into(),
            "d".into(),
        ])],
        &slots[..3],
    );
    emit_return_abi(&mut out, &[Value::I32("i".into())], &[]);
    emit_return_abi(&mut out, &[Value::I64("l".into(), "h".into())], &slots[..1]);
    emit_return_abi(&mut out, &[Value::F32("f".into())], &[]);
    emit_return_abi(&mut out, &[Value::F64("d".into())], &[]);

    let mut body = String::new();
    let values = call_result_values(
        &results,
        "call()",
        &slots,
        &mut body,
        Value::I32("p".into()),
        false,
    )
    .unwrap();
    assert_eq!(values.len(), results.len());
    let mut external_body = String::new();
    call_result_values(
        &[ValType::F32],
        "foreign()",
        &[],
        &mut external_body,
        Value::F32("f".into()),
        true,
    )
    .unwrap();
    assert!(external_body.contains("+foreign()"));
}

#[test]
fn error_helpers_reject_wrong_internal_value_types() {
    assert!(expect_i64(Value::I32("x".into())).is_err());
    assert!(expect_v128(Value::I32("x".into())).is_err());
    assert!(expect_f32(Value::I32("x".into())).is_err());
    assert!(expect_f64(Value::I32("x".into())).is_err());
    assert!(expect_float(Value::I32("x".into())).is_err());
    assert!(expect_i32_pair(Value::F32("x".into()), Value::I32("y".into())).is_err());
    assert!(
        panic::catch_unwind(|| {
            i64_compare(
                Value::I64("a".into(), "b".into()),
                Value::I64("c".into(), "d".into()),
                BinaryOp::I32Add,
            )
            .unwrap();
        })
        .is_err()
    );
}

#[test]
fn module_context_reports_invalid_synthetic_layouts() {
    let options = CompileOptions::default();

    for kind in [
        ImportKind::Table(TableType {
            table64: false,
            initial: 0,
            maximum: None,
            typed_function: None,
        }),
        ImportKind::Tag(0),
    ] {
        let mut module = Module::default();
        module.imports.push(import(kind));
        let context = ModuleCx::new(&module, &options).unwrap();
        assert!(context.emit_wrapper_prefix(&mut String::new()).is_err());
    }

    let mut huge = Module::default();
    huge.memories.push(MemoryType {
        memory64: false,
        shared: false,
        initial: 3,
        maximum: None,
        page_size_log2: 63,
    });
    let context = ModuleCx::new(&huge, &options).unwrap();
    assert!(context.emit_wrapper_prefix(&mut String::new()).is_err());

    let mut combined = Module::default();
    combined.memories.push(MemoryType {
        memory64: false,
        shared: false,
        initial: 1,
        maximum: None,
        page_size_log2: 32,
    });
    let context = ModuleCx::new(&combined, &options).unwrap();
    assert!(context.emit_wrapper_prefix(&mut String::new()).is_err());

    let module = Module::default();
    let context = ModuleCx::new(&module, &options).unwrap();
    assert!(context.function_type(0).is_err());
    assert_eq!(context.outer_offset(&ConstExpr::I32(-1)).unwrap(), "-1");
    assert_eq!(context.outer_offset(&ConstExpr::I64(1)).unwrap(), "1");
    assert_eq!(
        context.outer_offset(&ConstExpr::I64(1i64 << 32)).unwrap(),
        "-1"
    );
    assert!(context.outer_offset(&ConstExpr::RefNull).is_err());

    let mut imported_global = Module {
        imported_global_count: 1,
        ..Module::default()
    };
    imported_global.global_types.push(GlobalType {
        ty: ValType::I32,
        mutable: false,
    });
    let context = ModuleCx::new(&imported_global, &options).unwrap();
    assert_eq!(
        context.outer_offset(&ConstExpr::GlobalGet(0)).unwrap(),
        "f.ga"
    );

    let mut active_table = Module::default();
    active_table.elements.push(Element {
        mode: ElementMode::Active {
            table: 1,
            offset: ConstExpr::I32(0),
        },
        items: vec![],
    });
    let context = ModuleCx::new(&active_table, &options).unwrap();
    assert!(context.emit_outer_initializers(&mut String::new()).is_err());

    let mut invalid_dispatch = Module::default();
    invalid_dispatch.types.push(function_type(vec![], vec![]));
    let mut context = ModuleCx::new(&invalid_dispatch, &options).unwrap();
    context.indirect_types.insert(2);
    assert!(context.emit_dispatchers(&mut String::new()).is_err());

    let context = ModuleCx::new(&module, &options).unwrap();
    let malformed = [CompiledFunction {
        index: 0,
        code: "malformed".into(),
    }];
    assert!(
        context
            .emit_compiled_functions(&mut String::new(), &malformed)
            .is_err()
    );
}
