use super::*;
use crate::ir::{BlockSig, FuncType, Instr};

fn instruction(offset: usize, op: Op) -> Instr {
    Instr { offset, op }
}

fn module_with(function: Function, ty: FuncType) -> Module {
    Module {
        types: vec![ty],
        function_type_indices: vec![0],
        functions: vec![function],
        ..Module::default()
    }
}

#[test]
fn typed_value_graph_canonicalizes_rotated_byte_switches() {
    let targets = vec![0; 18];
    let function = Function {
        type_index: 0,
        locals: vec![ValType::I32],
        body: vec![
            instruction(0, Op::LocalGet(0)),
            instruction(1, Op::LocalTee(1)),
            instruction(2, Op::I32Const(7)),
            instruction(3, Op::Binary(BinaryOp::I32Shl)),
            instruction(4, Op::LocalGet(1)),
            instruction(5, Op::I32Const(254)),
            instruction(6, Op::Binary(BinaryOp::I32And)),
            instruction(7, Op::I32Const(1)),
            instruction(8, Op::Binary(BinaryOp::I32ShrU)),
            instruction(9, Op::Binary(BinaryOp::I32Or)),
            instruction(10, Op::I32Const(255)),
            instruction(11, Op::Binary(BinaryOp::I32And)),
            instruction(
                12,
                Op::BrTable {
                    targets,
                    default: 0,
                },
            ),
            instruction(13, Op::End),
        ],
        name: None,
    };
    let module = module_with(
        function,
        FuncType {
            params: vec![ValType::I32],
            results: vec![],
        },
    );
    let plan = optimize_function(&module, 0, &module.functions[0]).unwrap();
    assert_eq!(
        plan.switch(12),
        Some(SwitchDecision::RotateLocal { local: 1, left: 1 })
    );
}

#[test]
fn constant_propagation_plans_conditional_branches() {
    let function = Function {
        type_index: 0,
        locals: vec![],
        body: vec![
            instruction(0, Op::I32Const(0)),
            instruction(1, Op::BrIf(0)),
            instruction(2, Op::I32Const(20)),
            instruction(3, Op::I32Const(22)),
            instruction(4, Op::Binary(BinaryOp::I32Add)),
            instruction(5, Op::BrIf(0)),
            instruction(6, Op::End),
        ],
        name: None,
    };
    let module = module_with(
        function,
        FuncType {
            params: vec![],
            results: vec![],
        },
    );
    let plan = optimize_function(&module, 0, &module.functions[0]).unwrap();
    assert_eq!(plan.branch(1), Some(BranchDecision::Never));
    assert_eq!(plan.branch(5), Some(BranchDecision::Always));
    assert_eq!(plan.i32_constant(4), Some(42));
}

#[test]
fn constant_propagation_tracks_straight_line_local_assignments() {
    let function = Function {
        type_index: 0,
        locals: vec![ValType::I32],
        body: vec![
            instruction(0, Op::I32Const(20)),
            instruction(1, Op::LocalSet(0)),
            instruction(2, Op::LocalGet(0)),
            instruction(3, Op::I32Const(22)),
            instruction(4, Op::Binary(BinaryOp::I32Add)),
            instruction(5, Op::Drop),
            instruction(6, Op::End),
        ],
        name: None,
    };
    let module = module_with(
        function,
        FuncType {
            params: vec![],
            results: vec![],
        },
    );
    let plan = optimize_function(&module, 0, &module.functions[0]).unwrap();
    assert_eq!(plan.i32_constant(2), Some(20));
    assert_eq!(plan.i32_constant(4), Some(42));
}

#[test]
fn cfg_models_loop_and_block_branch_targets() {
    let empty = BlockSig {
        params: vec![],
        results: vec![],
    };
    let function = Function {
        type_index: 0,
        locals: vec![],
        body: vec![
            instruction(0, Op::Block(empty.clone())),
            instruction(1, Op::Loop(empty)),
            instruction(2, Op::I32Const(0)),
            instruction(3, Op::BrIf(0)),
            instruction(4, Op::End),
            instruction(5, Op::End),
            instruction(6, Op::End),
        ],
        name: None,
    };
    let module = module_with(
        function,
        FuncType {
            params: vec![],
            results: vec![],
        },
    );
    let mut mir = FunctionMir::build(&module, 0, &module.functions[0]).unwrap();
    mir.verify(module.functions[0].body.len()).unwrap();
    mark_reachable(&mut mir);
    assert!(mir.blocks.iter().all(|block| block.reachable));

    let branch_block = mir.instruction_blocks[3];
    let loop_body = mir.instruction_blocks[2];
    let Terminator::Branch { then_block, .. } = mir.blocks[branch_block.0].terminator else {
        panic!("expected loop conditional branch");
    };
    assert_eq!(then_block, Some(loop_body));
}

#[test]
fn repeated_signed_division_in_loops_uses_reciprocal_cache() {
    let empty = BlockSig {
        params: vec![],
        results: vec![],
    };
    let function = Function {
        type_index: 0,
        locals: vec![],
        body: vec![
            instruction(0, Op::Loop(empty)),
            instruction(1, Op::LocalGet(0)),
            instruction(2, Op::LocalGet(1)),
            instruction(3, Op::Binary(BinaryOp::I32DivS)),
            instruction(4, Op::Drop),
            instruction(5, Op::LocalGet(2)),
            instruction(6, Op::BrIf(0)),
            instruction(7, Op::End),
            instruction(8, Op::LocalGet(0)),
            instruction(9, Op::LocalGet(1)),
            instruction(10, Op::Binary(BinaryOp::I32DivS)),
            instruction(11, Op::Drop),
            instruction(12, Op::End),
        ],
        name: None,
    };
    let module = module_with(
        function,
        FuncType {
            params: vec![ValType::I32, ValType::I32, ValType::I32],
            results: vec![],
        },
    );
    let plan = optimize_function(&module, 0, &module.functions[0]).unwrap();
    assert!(plan.reciprocal_division(3));
    assert!(!plan.reciprocal_division(10));
}

#[test]
fn i64_demand_analysis_distinguishes_low_only_and_full_consumers() {
    let low_only_function = Function {
        type_index: 0,
        locals: vec![],
        body: vec![
            instruction(0, Op::I64Const(70_000)),
            instruction(1, Op::I64Const(3)),
            instruction(2, Op::Binary(BinaryOp::I64Mul)),
            instruction(3, Op::Unary(UnaryOp::I32WrapI64)),
            instruction(4, Op::Drop),
            instruction(5, Op::End),
        ],
        name: None,
    };
    let low_only_module = module_with(
        low_only_function,
        FuncType {
            params: vec![],
            results: vec![],
        },
    );
    let plan = optimize_function(&low_only_module, 0, &low_only_module.functions[0]).unwrap();
    assert!(plan.i64_low_only(2));

    let full_function = Function {
        type_index: 0,
        locals: vec![],
        body: vec![
            instruction(0, Op::I64Const(70_000)),
            instruction(1, Op::I64Const(3)),
            instruction(2, Op::Binary(BinaryOp::I64Mul)),
            instruction(3, Op::I64Const(210_000)),
            instruction(4, Op::Binary(BinaryOp::I64Eq)),
            instruction(5, Op::Drop),
            instruction(6, Op::End),
        ],
        name: None,
    };
    let full_module = module_with(
        full_function,
        FuncType {
            params: vec![],
            results: vec![],
        },
    );
    let plan = optimize_function(&full_module, 0, &full_module.functions[0]).unwrap();
    assert!(!plan.i64_low_only(2));
}
