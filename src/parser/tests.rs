use super::*;
use wasmparser::{
    AbstractHeapType, BinaryReader, BlockType, Catch as WasmCatch, HeapType, Operator,
    OperatorsReader, RefType, TableType as WasmTableType, TryTable as WasmTryTable, UnpackedIndex,
    ValType as WasmValType,
};

fn abstract_ref(ty: AbstractHeapType) -> RefType {
    RefType::new(false, HeapType::Abstract { shared: false, ty }).unwrap()
}

#[test]
fn type_conversion_and_feature_recording_cover_all_supported_shapes() {
    let mut features = FeatureUse::default();
    for ty in [
        ValType::I32,
        ValType::I64,
        ValType::F32,
        ValType::F64,
        ValType::V128,
        ValType::FuncRef(None),
        ValType::FuncRef(Some(2)),
    ] {
        record_type_feature(ty, &mut features);
    }
    assert!(features.i64);
    assert!(features.simd);
    assert!(features.reference_types);
    assert!(features.typed_references);

    assert_eq!(convert_type(WasmValType::I32).unwrap(), ValType::I32);
    assert!(convert_type(WasmValType::Ref(RefType::EXTERNREF)).is_err());

    let types = [FuncType {
        params: vec![ValType::I32],
        results: vec![ValType::F64],
    }];
    assert_eq!(sig(BlockType::FuncType(0), &types).unwrap().params.len(), 1);
    assert!(sig(BlockType::FuncType(1), &types).is_err());
    assert!(sig(BlockType::Type(WasmValType::EXTERNREF), &types).is_err());

    assert_eq!(
        heap_type_index(HeapType::Concrete(UnpackedIndex::Module(3))),
        Some(3)
    );
    assert_eq!(
        heap_type_index(HeapType::Exact(UnpackedIndex::Module(4))),
        Some(4)
    );
    assert_eq!(
        heap_type_index(HeapType::Abstract {
            shared: false,
            ty: AbstractHeapType::Func,
        }),
        None
    );
}

#[test]
fn table_conversion_covers_abstract_concrete_exact_and_invalid_references() {
    let mut features = FeatureUse::default();
    let plain = convert_table(
        WasmTableType {
            element_type: RefType::FUNCREF,
            table64: false,
            initial: 1,
            maximum: Some(2),
            shared: false,
        },
        &mut features,
    )
    .unwrap();
    assert_eq!(plain.typed_function, None);

    let concrete_ref = RefType::new(true, HeapType::Concrete(UnpackedIndex::Module(7))).unwrap();
    let concrete = convert_table(
        WasmTableType {
            element_type: concrete_ref,
            table64: true,
            initial: 2,
            maximum: None,
            shared: true,
        },
        &mut features,
    )
    .unwrap();
    assert_eq!(concrete.typed_function, Some(7));
    assert!(concrete.table64);
    assert!(features.memory64 && features.threads && features.typed_references);

    let exact_ref = RefType::new(false, HeapType::Exact(UnpackedIndex::Module(8))).unwrap();
    assert_eq!(
        convert_table(
            WasmTableType {
                element_type: exact_ref,
                table64: false,
                initial: 0,
                maximum: None,
                shared: false,
            },
            &mut features,
        )
        .unwrap()
        .typed_function,
        Some(8)
    );

    assert!(
        convert_table(
            WasmTableType {
                element_type: abstract_ref(AbstractHeapType::Extern),
                table64: false,
                initial: 0,
                maximum: None,
                shared: false,
            },
            &mut features,
        )
        .is_err()
    );
}

#[test]
fn operator_conversion_classifies_supported_and_unhandled_proposals() {
    let types = [FuncType {
        params: vec![],
        results: vec![],
    }];
    let concrete = HeapType::Concrete(UnpackedIndex::Module(0));
    let mut features = FeatureUse::default();

    assert!(matches!(
        convert_operator(
            Operator::ReturnCallIndirect {
                type_index: 0,
                table_index: 1,
            },
            &types,
            &mut features,
        )
        .unwrap(),
        Op::ReturnCallIndirect { .. }
    ));
    assert!(matches!(
        convert_operator(
            Operator::RefTestNonNull { hty: concrete },
            &types,
            &mut features
        )
        .unwrap(),
        Op::RefTest(Some(0))
    ));
    assert!(matches!(
        convert_operator(
            Operator::RefCastNullable { hty: concrete },
            &types,
            &mut features
        )
        .unwrap(),
        Op::RefCast(Some(0))
    ));
    assert!(matches!(
        convert_operator(Operator::I64ExtendI32S, &types, &mut features).unwrap(),
        Op::Unary(UnaryOp::I64ExtendI32S)
    ));
    assert!(matches!(
        convert_operator(Operator::I64ExtendI32U, &types, &mut features).unwrap(),
        Op::Unary(UnaryOp::I64ExtendI32U)
    ));

    for operator in [
        Operator::AtomicFence,
        Operator::StructNew {
            struct_type_index: 0,
        },
        Operator::I8x16Add,
        Operator::ThrowRef,
        Operator::ReturnCallRef { type_index: 0 },
        Operator::MemoryDiscard { mem: 0 },
    ] {
        assert!(matches!(
            convert_operator(operator, &types, &mut features).unwrap(),
            Op::Unsupported { .. }
        ));
    }
    assert!(features.threads);
    assert!(features.gc);
    assert!(features.simd);
    assert!(features.exceptions);
    assert!(features.typed_references);
    assert!(!features.unsupported.is_empty());
}

#[test]
fn constant_expression_parser_accepts_every_supported_encoding_and_rejects_invalid_sequences() {
    fn parse(bytes: &[u8]) -> Result<ConstExpr, CompileError> {
        parse_const(OperatorsReader::new(BinaryReader::new(bytes, 0)))
    }

    assert!(matches!(
        parse(&[0x41, 0x7f, 0x0b]).unwrap(),
        ConstExpr::I32(-1)
    ));
    assert!(matches!(
        parse(&[0x42, 0x7e, 0x0b]).unwrap(),
        ConstExpr::I64(-2)
    ));
    assert!(matches!(
        parse(&[0x43, 0x00, 0x00, 0xc0, 0x3f, 0x0b]).unwrap(),
        ConstExpr::F32(_)
    ));
    assert!(matches!(
        parse(&[0x44, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x04, 0x40, 0x0b]).unwrap(),
        ConstExpr::F64(_)
    ));
    assert!(matches!(
        parse(&[0x23, 0x00, 0x0b]).unwrap(),
        ConstExpr::GlobalGet(0)
    ));
    assert!(matches!(
        parse(&[0xd0, 0x70, 0x0b]).unwrap(),
        ConstExpr::RefNull
    ));
    assert!(matches!(
        parse(&[0xd2, 0x00, 0x0b]).unwrap(),
        ConstExpr::RefFunc(0)
    ));
    assert!(parse(&[0x6a, 0x0b]).is_err());
    assert!(parse(&[0x41, 0x00, 0x01, 0x0b]).is_err());
}

#[test]
fn capability_checks_cover_gc_and_generic_unsupported_proposals() {
    let options = CompileOptions::default();
    let mut module = Module::default();
    module.features.gc = true;
    assert!(
        check_capabilities(&module, &options)
            .unwrap_err()
            .message
            .contains("garbage collection")
    );

    module.features.gc = false;
    module.features.unsupported.push("MemoryDiscard".into());
    assert!(
        check_capabilities(&module, &options)
            .unwrap_err()
            .message
            .contains("unsupported proposal")
    );
}

#[test]
fn tail_call_classification_covers_direct_indirect_recursive_and_invalid_edges() {
    fn function(ops: Vec<Op>) -> Function {
        Function {
            type_index: 0,
            locals: vec![],
            body: ops.into_iter().map(|op| Instr { offset: 0, op }).collect(),
            name: None,
        }
    }

    let mut recursive = Module::default();
    recursive.features.tail_call = true;
    recursive.function_type_indices = vec![0, 0];
    recursive.functions = vec![
        function(vec![Op::Call(1), Op::ReturnCall(1)]),
        function(vec![Op::Call(0)]),
    ];
    classify_recursive_tail_calls(&mut recursive);
    assert!(recursive.features.tail_call);

    let mut indirect = Module::default();
    indirect.features.tail_call = true;
    indirect.function_type_indices = vec![0];
    indirect.functions = vec![function(vec![Op::ReturnCallIndirect {
        type_index: 0,
        table_index: 0,
    }])];
    classify_recursive_tail_calls(&mut indirect);
    assert!(indirect.features.tail_call);

    let mut invalid_edge = Module::default();
    invalid_edge.features.tail_call = true;
    invalid_edge.function_type_indices = vec![0];
    invalid_edge.functions = vec![function(vec![Op::ReturnCall(9)])];
    classify_recursive_tail_calls(&mut invalid_edge);
    assert!(!invalid_edge.features.tail_call);
}

#[test]
fn try_table_conversion_maps_portable_catches_and_rejects_exception_references() {
    let mut features = FeatureUse::default();
    let converted = convert_operator(
        Operator::TryTable {
            try_table: WasmTryTable {
                ty: BlockType::Empty,
                catches: vec![
                    WasmCatch::One { tag: 2, label: 3 },
                    WasmCatch::All { label: 4 },
                ],
            },
        },
        &[],
        &mut features,
    )
    .unwrap();
    assert!(features.exceptions);
    assert!(matches!(
        converted,
        Op::TryTable {
            sig: BlockSig { ref params, ref results },
            ref catches,
        } if params.is_empty()
            && results.is_empty()
            && matches!(
                catches.as_slice(),
                [CatchClause::Tag { tag: 2, label: 3 }, CatchClause::All { label: 4 }]
            )
    ));

    for catch in [
        WasmCatch::OneRef { tag: 0, label: 0 },
        WasmCatch::AllRef { label: 0 },
    ] {
        let error = convert_operator(
            Operator::TryTable {
                try_table: WasmTryTable {
                    ty: BlockType::Empty,
                    catches: vec![catch],
                },
            },
            &[],
            &mut FeatureUse::default(),
        )
        .unwrap_err();
        assert_eq!(error.kind, ErrorKind::UnsupportedFeature);
        assert!(error.message.contains("exception reference identity"));
    }
}

#[test]
fn module_parser_covers_component_and_tag_boundaries() {
    let component = wat::parse_str("(component)").unwrap();
    let component_error = parse_module(&component, &CompileOptions::default()).unwrap_err();
    assert_eq!(component_error.kind, ErrorKind::InvalidInput);
    assert!(
        component_error
            .message
            .contains("invalid WebAssembly input")
    );

    let declarations = wat::parse_str(
        r#"(module
            (type $empty (func))
            (import "env" "table" (table 1 funcref))
            (import "env" "tag" (tag (type $empty)))
            (tag $local (type $empty))
            (export "tag" (tag $local)))"#,
    )
    .unwrap();
    let declaration_error = parse_module(&declarations, &CompileOptions::default()).unwrap_err();
    assert_eq!(declaration_error.kind, ErrorKind::UnsupportedFeature);
    assert!(
        declaration_error
            .message
            .contains("imported or exported tables")
    );

    let tags = wat::parse_str(
        r#"(module
            (type $empty (func))
            (import "env" "tag" (tag (type $empty)))
            (tag $local (type $empty))
            (export "tag" (tag $local)))"#,
    )
    .unwrap();
    let tag_error = parse_module(&tags, &CompileOptions::default()).unwrap_err();
    assert_eq!(tag_error.kind, ErrorKind::UnsupportedFeature);
    assert!(tag_error.message.contains("exception handling"));
}

#[test]
fn module_parser_accepts_expression_elements_in_every_mode() {
    let wasm = wat::parse_str(
        r#"(module
            (type $empty (func))
            (func $target (type $empty))
            (table 3 funcref)
            (elem (table 0) (i32.const 0) funcref
                (ref.func $target) (ref.null func))
            (elem funcref (ref.null func))
            (elem declare funcref (ref.func $target)))"#,
    )
    .unwrap();
    let module = parse_module(&wasm, &CompileOptions::default()).unwrap();
    assert_eq!(module.elements.len(), 3);
    assert!(matches!(
        module.elements[0].mode,
        ElementMode::Active { table: 0, .. }
    ));
    assert!(matches!(module.elements[1].mode, ElementMode::Passive));
    assert!(matches!(module.elements[2].mode, ElementMode::Declared));
    assert_eq!(module.elements[0].items, vec![Some(0), None]);
    assert_eq!(module.elements[1].items, vec![None]);
    assert_eq!(module.elements[2].items, vec![Some(0)]);
}

#[test]
fn module_parser_reports_gc_and_invalid_element_expression_shapes() {
    let gc = wat::parse_str("(module (type (struct (field i32))))").unwrap();
    let gc_error = parse_module(&gc, &CompileOptions::default()).unwrap_err();
    assert_eq!(gc_error.kind, ErrorKind::UnsupportedFeature);
    assert!(gc_error.message.contains("garbage collection"));

    let non_function = wat::parse_str("(module (elem externref (ref.null extern)))").unwrap();
    let non_function_error = parse_module(&non_function, &CompileOptions::default()).unwrap_err();
    assert_eq!(non_function_error.kind, ErrorKind::UnsupportedFeature);
    assert!(
        non_function_error
            .message
            .contains("non-function element segments")
    );

    let global_get = wat::parse_str(
        r#"(module
            (import "env" "value" (global funcref))
            (table 1 funcref)
            (elem (i32.const 0) funcref (global.get 0)))"#,
    )
    .unwrap();
    let global_get_error = parse_module(&global_get, &CompileOptions::default()).unwrap_err();
    assert_eq!(global_get_error.kind, ErrorKind::UnsupportedFeature);
    assert!(
        global_get_error
            .message
            .contains("element item is not ref.func or ref.null")
    );

    let initialized_table = wat::parse_str(
        r#"(module
            (type $empty (func))
            (func $target (type $empty))
            (elem declare func $target)
            (table 1 funcref (ref.func $target)))"#,
    )
    .unwrap();
    let module = parse_module(&initialized_table, &CompileOptions::default()).unwrap();
    assert!(module.features.reference_types);
}
