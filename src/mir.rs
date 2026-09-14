use crate::diagnostics::{CompileError, ErrorKind};
use crate::ir::{BinaryOp, Function, LoadOp, MemArg, Module, Op, SimdOp, UnaryOp, ValType};
use std::collections::{BTreeSet, VecDeque};
use std::ops::Range;

#[cfg(test)]
#[path = "mir/tests.rs"]
mod tests;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct BlockId(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
struct ValueId(usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BranchDecision {
    Always,
    Never,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SwitchDecision {
    Constant(u32),
    RotateLocal { local: u32, left: u32 },
}

#[derive(Debug, Clone)]
pub(crate) struct FunctionPlan {
    branches: Vec<Option<BranchDecision>>,
    switches: Vec<Option<SwitchDecision>>,
    reciprocal_divisions: Vec<bool>,
    i32_constants: Vec<Option<i32>>,
}

impl FunctionPlan {
    pub(crate) fn branch(&self, instruction: usize) -> Option<BranchDecision> {
        self.branches.get(instruction).copied().flatten()
    }

    pub(crate) fn switch(&self, instruction: usize) -> Option<SwitchDecision> {
        self.switches.get(instruction).copied().flatten()
    }

    pub(crate) fn reciprocal_division(&self, instruction: usize) -> bool {
        self.reciprocal_divisions
            .get(instruction)
            .copied()
            .unwrap_or(false)
    }

    pub(crate) fn i32_constant(&self, instruction: usize) -> Option<i32> {
        self.i32_constants.get(instruction).copied().flatten()
    }
}

#[derive(Debug, Clone, Copy)]
enum ValueDef {
    I32Const(i32),
    LocalGet {
        local: u32,
        version: u32,
        alias: Option<ValueId>,
    },
    Unary(UnaryOp, ValueId),
    Binary(BinaryOp, ValueId, ValueId),
    Load(LoadOp, ValueId, MemArg),
    Opaque,
}

#[derive(Debug, Clone, Copy)]
struct ValueData {
    ty: ValType,
    def: ValueDef,
    source: usize,
}

#[derive(Debug, Clone)]
enum Terminator {
    Fallthrough(Option<BlockId>),
    Jump(Option<BlockId>),
    Branch {
        then_block: Option<BlockId>,
        else_block: Option<BlockId>,
    },
    Switch {
        targets: Vec<Option<BlockId>>,
        default: Option<BlockId>,
    },
    Return,
    Unreachable,
}

impl Terminator {
    fn successors(&self) -> impl Iterator<Item = BlockId> + '_ {
        let mut successors = Vec::new();
        match self {
            Self::Fallthrough(target) | Self::Jump(target) => {
                successors.extend(*target);
            }
            Self::Branch {
                then_block,
                else_block,
            } => {
                successors.extend(*then_block);
                if else_block != then_block {
                    successors.extend(*else_block);
                }
            }
            Self::Switch { targets, default } => {
                successors.extend(targets.iter().flatten().copied());
                successors.extend(*default);
            }
            Self::Return | Self::Unreachable => {}
        }
        successors.into_iter()
    }
}

#[derive(Debug, Clone)]
struct BasicBlock {
    instructions: Range<usize>,
    terminator: Terminator,
    reachable: bool,
}

#[derive(Debug, Clone)]
struct FunctionMir {
    blocks: Vec<BasicBlock>,
    instruction_blocks: Vec<BlockId>,
    values: Vec<ValueData>,
    control_values: Vec<Option<ValueId>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegionKind {
    Block,
    Loop,
    If,
}

#[derive(Debug, Clone)]
struct Region {
    kind: RegionKind,
    start: usize,
    else_at: Option<usize>,
    end: usize,
}

#[derive(Debug, Clone, Copy)]
struct ActiveRegion {
    region: usize,
    parent: Option<usize>,
}

#[derive(Debug)]
struct RegionLayout {
    regions: Vec<Region>,
    opener_regions: Vec<Option<usize>>,
    active_heads: Vec<Option<usize>>,
    active_nodes: Vec<ActiveRegion>,
}

#[derive(Debug, Clone, Copy)]
enum Pass {
    MarkReachable,
    ConstantPropagation,
    CanonicalizeSwitches,
    PlanReciprocalDivisions,
}

struct PassManager {
    passes: [Pass; 4],
}

impl Default for PassManager {
    fn default() -> Self {
        Self {
            passes: [
                Pass::MarkReachable,
                Pass::ConstantPropagation,
                Pass::CanonicalizeSwitches,
                Pass::PlanReciprocalDivisions,
            ],
        }
    }
}

impl PassManager {
    fn run(&self, mir: &mut FunctionMir, function: &Function) -> FunctionPlan {
        let mut plan = FunctionPlan {
            branches: vec![None; function.body.len()],
            switches: vec![None; function.body.len()],
            reciprocal_divisions: vec![false; function.body.len()],
            i32_constants: vec![None; function.body.len()],
        };
        let mut constants = Vec::new();
        for pass in self.passes {
            match pass {
                Pass::MarkReachable => mark_reachable(mir),
                Pass::ConstantPropagation => {
                    constants = propagate_constants(mir);
                    for (value, constant) in mir.values.iter().zip(&constants) {
                        if value.ty == ValType::I32 {
                            plan.i32_constants[value.source] = *constant;
                        }
                    }
                    plan_constant_branches(mir, function, &constants, &mut plan);
                }
                Pass::CanonicalizeSwitches => {
                    plan_switch_canonicalization(mir, function, &constants, &mut plan)
                }
                Pass::PlanReciprocalDivisions => {
                    plan_loop_reciprocal_divisions(mir, function, &mut plan)
                }
            }
        }
        plan
    }
}

pub(crate) fn optimize_function(
    module: &Module,
    function_index: usize,
    function: &Function,
) -> Result<FunctionPlan, CompileError> {
    let mut mir = FunctionMir::build(module, function_index, function)?;
    mir.verify(function.body.len())?;
    Ok(PassManager::default().run(&mut mir, function))
}

impl FunctionMir {
    fn build(
        module: &Module,
        function_index: usize,
        function: &Function,
    ) -> Result<Self, CompileError> {
        let layout = build_region_layout(function)?;
        let (blocks, instruction_blocks) = build_cfg(function, &layout)?;
        let mut mir = Self {
            blocks,
            instruction_blocks,
            values: Vec::new(),
            control_values: vec![None; function.body.len()],
        };
        mir.build_values(module, function_index, function)?;
        Ok(mir)
    }

    fn push_value(&mut self, ty: ValType, def: ValueDef, source: usize) -> ValueId {
        let id = ValueId(self.values.len());
        self.values.push(ValueData { ty, def, source });
        id
    }

    fn opaque(&mut self, ty: ValType, source: usize) -> ValueId {
        self.push_value(ty, ValueDef::Opaque, source)
    }

    fn build_values(
        &mut self,
        module: &Module,
        function_index: usize,
        function: &Function,
    ) -> Result<(), CompileError> {
        let function_type = module_function_type(module, function_index)?;
        let mut local_types = function_type.params.clone();
        local_types.extend_from_slice(&function.locals);
        let mut local_versions = vec![0u32; local_types.len()];
        for block_index in 0..self.blocks.len() {
            let instructions = self.blocks[block_index].instructions.clone();
            let mut stack = Vec::<ValueId>::new();
            let mut local_values = vec![None; local_types.len()];
            for instruction_index in instructions {
                let op = &function.body[instruction_index].op;
                match op {
                    Op::I32Const(value) => {
                        let value = self.push_value(
                            ValType::I32,
                            ValueDef::I32Const(*value),
                            instruction_index,
                        );
                        stack.push(value);
                    }
                    Op::I64Const(_) => {
                        let value = self.opaque(ValType::I64, instruction_index);
                        stack.push(value);
                    }
                    Op::F32Const(_) => {
                        let value = self.opaque(ValType::F32, instruction_index);
                        stack.push(value);
                    }
                    Op::F64Const(_) => {
                        let value = self.opaque(ValType::F64, instruction_index);
                        stack.push(value);
                    }
                    Op::LocalGet(local) => {
                        let ty = local_types.get(*local as usize).copied().ok_or_else(|| {
                            internal(format!("MIR local index {local} is out of range"))
                        })?;
                        let version = local_versions[*local as usize];
                        let value = self.push_value(
                            ty,
                            ValueDef::LocalGet {
                                local: *local,
                                version,
                                alias: local_values[*local as usize],
                            },
                            instruction_index,
                        );
                        stack.push(value);
                    }
                    Op::GlobalGet(global) => {
                        let ty = module
                            .global_types
                            .get(*global as usize)
                            .map(|global| global.ty)
                            .ok_or_else(|| {
                                internal(format!("MIR global index {global} is out of range"))
                            })?;
                        let value = self.opaque(ty, instruction_index);
                        stack.push(value);
                    }
                    Op::Unary(op) => {
                        let operand = pop_or_opaque(
                            self,
                            &mut stack,
                            unary_operand_type(*op),
                            instruction_index,
                        );
                        let value = self.push_value(
                            unary_result_type(*op),
                            ValueDef::Unary(*op, operand),
                            instruction_index,
                        );
                        stack.push(value);
                    }
                    Op::Binary(op) => {
                        let operand_ty = binary_operand_type(*op);
                        let right = pop_or_opaque(self, &mut stack, operand_ty, instruction_index);
                        let left = pop_or_opaque(self, &mut stack, operand_ty, instruction_index);
                        let value = self.push_value(
                            binary_result_type(*op),
                            ValueDef::Binary(*op, left, right),
                            instruction_index,
                        );
                        stack.push(value);
                    }
                    Op::LocalSet(index) => {
                        let value = stack.pop();
                        let version = local_versions.get_mut(*index as usize).ok_or_else(|| {
                            internal(format!("MIR local index {index} is out of range"))
                        })?;
                        *version = version
                            .checked_add(1)
                            .ok_or_else(|| internal("MIR local version overflow"))?;
                        local_values[*index as usize] = value;
                    }
                    Op::GlobalSet(_) | Op::Drop => {
                        stack.pop();
                    }
                    Op::LocalTee(index) => {
                        let value = stack
                            .pop()
                            .unwrap_or_else(|| self.opaque(ValType::I32, instruction_index));
                        let ty = self.values[value.0].ty;
                        let version = local_versions.get_mut(*index as usize).ok_or_else(|| {
                            internal(format!("MIR local index {index} is out of range"))
                        })?;
                        *version = version
                            .checked_add(1)
                            .ok_or_else(|| internal("MIR local version overflow"))?;
                        local_values[*index as usize] = Some(value);
                        let alias = self.push_value(
                            ty,
                            ValueDef::LocalGet {
                                local: *index,
                                version: *version,
                                alias: Some(value),
                            },
                            instruction_index,
                        );
                        stack.push(alias);
                    }
                    Op::If(_) | Op::BrIf(_) | Op::BrTable { .. } => {
                        let value =
                            pop_or_opaque(self, &mut stack, ValType::I32, instruction_index);
                        self.control_values[instruction_index] = Some(value);
                    }
                    Op::Select(ty) => {
                        stack.pop();
                        let right = stack.pop();
                        let left = stack.pop();
                        let result_ty = ty
                            .or_else(|| left.map(|value| self.values[value.0].ty))
                            .or_else(|| right.map(|value| self.values[value.0].ty))
                            .unwrap_or(ValType::I32);
                        let value = self.opaque(result_ty, instruction_index);
                        stack.push(value);
                    }
                    Op::Load(op, arg) => {
                        let address = pop_or_opaque(
                            self,
                            &mut stack,
                            memory_index_type(module, arg.memory)?,
                            instruction_index,
                        );
                        let value = self.push_value(
                            load_result_type(*op),
                            ValueDef::Load(*op, address, *arg),
                            instruction_index,
                        );
                        stack.push(value);
                    }
                    Op::Store(_, _) => {
                        stack.pop();
                        stack.pop();
                    }
                    Op::MemorySize(memory) => {
                        let ty = memory_index_type(module, *memory)?;
                        let value = self.opaque(ty, instruction_index);
                        stack.push(value);
                    }
                    Op::MemoryGrow(memory) => {
                        stack.pop();
                        let ty = memory_index_type(module, *memory)?;
                        let value = self.opaque(ty, instruction_index);
                        stack.push(value);
                    }
                    Op::MemoryInit { .. } | Op::MemoryCopy { .. } => {
                        pop_many(&mut stack, 3);
                    }
                    Op::MemoryFill(_) => pop_many(&mut stack, 3),
                    Op::DataDrop(_) | Op::ElemDrop(_) => {}
                    Op::TableGet(table) => {
                        stack.pop();
                        let ty = module
                            .tables
                            .get(*table as usize)
                            .map(|table| ValType::FuncRef(table.typed_function))
                            .ok_or_else(|| {
                                internal(format!("MIR table index {table} is out of range"))
                            })?;
                        let value = self.opaque(ty, instruction_index);
                        stack.push(value);
                    }
                    Op::TableSet(_) => pop_many(&mut stack, 2),
                    Op::TableSize(table) => {
                        let ty = table_index_type(module, *table)?;
                        let value = self.opaque(ty, instruction_index);
                        stack.push(value);
                    }
                    Op::TableGrow(table) => {
                        pop_many(&mut stack, 2);
                        let ty = table_index_type(module, *table)?;
                        let value = self.opaque(ty, instruction_index);
                        stack.push(value);
                    }
                    Op::TableFill(_) | Op::TableCopy { .. } | Op::TableInit { .. } => {
                        pop_many(&mut stack, 3);
                    }
                    Op::RefNull(ty) => {
                        let value = self.opaque(ValType::FuncRef(*ty), instruction_index);
                        stack.push(value);
                    }
                    Op::RefFunc(_) => {
                        let value = self.opaque(ValType::FuncRef(None), instruction_index);
                        stack.push(value);
                    }
                    Op::RefIsNull | Op::RefTest(_) => {
                        stack.pop();
                        let value = self.opaque(ValType::I32, instruction_index);
                        stack.push(value);
                    }
                    Op::RefCast(ty) => {
                        stack.pop();
                        let value = self.opaque(ValType::FuncRef(*ty), instruction_index);
                        stack.push(value);
                    }
                    Op::Call(index) => {
                        apply_call(
                            self,
                            module_function_type(module, *index as usize)?,
                            &mut stack,
                            instruction_index,
                        );
                    }
                    Op::ReturnCall(index) => {
                        apply_call(
                            self,
                            module_function_type(module, *index as usize)?,
                            &mut stack,
                            instruction_index,
                        );
                    }
                    Op::CallIndirect { type_index, .. }
                    | Op::ReturnCallIndirect { type_index, .. }
                    | Op::CallRef(type_index) => {
                        stack.pop();
                        let ty = module.types.get(*type_index as usize).ok_or_else(|| {
                            internal(format!("MIR type index {type_index} is out of range"))
                        })?;
                        apply_call(self, ty, &mut stack, instruction_index);
                    }
                    Op::Simd(op) => apply_simd(self, op, &mut stack, instruction_index),
                    Op::Block(_)
                    | Op::Loop(_)
                    | Op::Else
                    | Op::End
                    | Op::Br(_)
                    | Op::Return
                    | Op::Unreachable
                    | Op::Nop
                    | Op::Throw(_)
                    | Op::TryTable { .. }
                    | Op::Unsupported { .. } => {}
                }
            }
        }
        Ok(())
    }

    fn verify(&self, instruction_count: usize) -> Result<(), CompileError> {
        if self.control_values.len() != instruction_count
            || self.instruction_blocks.len() != instruction_count
        {
            return Err(internal("MIR instruction maps have inconsistent lengths"));
        }
        let mut next_instruction = 0usize;
        for (index, block) in self.blocks.iter().enumerate() {
            if block.instructions.start != next_instruction
                || block.instructions.start >= block.instructions.end
                || block.instructions.end > instruction_count
            {
                return Err(internal(format!("MIR block {index} has an invalid range")));
            }
            for instruction in block.instructions.clone() {
                if self.instruction_blocks[instruction] != BlockId(index) {
                    return Err(internal(format!(
                        "MIR instruction {instruction} has an invalid block mapping"
                    )));
                }
            }
            for successor in block.terminator.successors() {
                if successor.0 >= self.blocks.len() {
                    return Err(internal(format!(
                        "MIR block {index} has an invalid successor"
                    )));
                }
            }
            next_instruction = block.instructions.end;
        }
        if next_instruction != instruction_count {
            return Err(internal("MIR blocks do not cover the function body"));
        }
        for (index, value) in self.values.iter().enumerate() {
            let verify_operand = |operand: ValueId| {
                if operand.0 >= index {
                    Err(internal(format!(
                        "MIR value {index} references a non-dominating value"
                    )))
                } else {
                    Ok(())
                }
            };
            match value.def {
                ValueDef::Unary(_, operand) => verify_operand(operand)?,
                ValueDef::LocalGet {
                    alias: Some(operand),
                    ..
                } => verify_operand(operand)?,
                ValueDef::Load(op, operand, arg) => {
                    verify_operand(operand)?;
                    if arg.align > load_natural_alignment(op) {
                        return Err(internal(format!(
                            "MIR load at {} has invalid alignment {}",
                            value.source, arg.align
                        )));
                    }
                    let _ = (arg.offset, arg.memory);
                }
                ValueDef::Binary(_, left, right) => {
                    verify_operand(left)?;
                    verify_operand(right)?;
                }
                ValueDef::I32Const(_)
                | ValueDef::LocalGet { alias: None, .. }
                | ValueDef::Opaque => {}
            }
            if value.source >= instruction_count {
                return Err(internal(format!("MIR value {index} has an invalid source")));
            }
        }
        Ok(())
    }
}

fn build_region_layout(function: &Function) -> Result<RegionLayout, CompileError> {
    let mut regions = Vec::<Region>::new();
    let mut opener_regions = vec![None; function.body.len()];
    let mut stack = Vec::<usize>::new();
    for (index, instruction) in function.body.iter().enumerate() {
        let kind = match instruction.op {
            Op::Block(_) => Some(RegionKind::Block),
            Op::Loop(_) => Some(RegionKind::Loop),
            Op::If(_) => Some(RegionKind::If),
            _ => None,
        };
        if let Some(kind) = kind {
            let region = regions.len();
            regions.push(Region {
                kind,
                start: index,
                else_at: None,
                end: usize::MAX,
            });
            opener_regions[index] = Some(region);
            stack.push(region);
            continue;
        }
        match instruction.op {
            Op::Else => {
                let region = *stack
                    .last()
                    .ok_or_else(|| internal("MIR encountered else without a region"))?;
                if regions[region].kind != RegionKind::If || regions[region].else_at.is_some() {
                    return Err(internal("MIR encountered an invalid else"));
                }
                regions[region].else_at = Some(index);
            }
            Op::End if !stack.is_empty() => {
                let region = stack.pop().unwrap();
                regions[region].end = index;
            }
            _ => {}
        }
    }
    if !stack.is_empty() || regions.iter().any(|region| region.end == usize::MAX) {
        return Err(internal("MIR encountered an unterminated control region"));
    }

    let mut active_heads = vec![None; function.body.len()];
    let mut active_nodes = Vec::<ActiveRegion>::with_capacity(regions.len());
    let mut head = None;
    for (index, instruction) in function.body.iter().enumerate() {
        active_heads[index] = head;
        if let Some(region) = opener_regions[index] {
            let node = active_nodes.len();
            active_nodes.push(ActiveRegion {
                region,
                parent: head,
            });
            head = Some(node);
        } else if matches!(instruction.op, Op::End) {
            if let Some(node) = head {
                head = active_nodes[node].parent;
            }
        }
    }
    Ok(RegionLayout {
        regions,
        opener_regions,
        active_heads,
        active_nodes,
    })
}

fn build_cfg(
    function: &Function,
    layout: &RegionLayout,
) -> Result<(Vec<BasicBlock>, Vec<BlockId>), CompileError> {
    let instruction_count = function.body.len();
    if instruction_count == 0 {
        return Err(internal("MIR cannot build an empty function body"));
    }
    let mut boundaries = BTreeSet::from([0usize, instruction_count]);
    for region in &layout.regions {
        boundaries.insert(region.start + 1);
        boundaries.insert(region.end + 1);
        if let Some(else_at) = region.else_at {
            boundaries.insert(else_at + 1);
        }
    }
    for (index, instruction) in function.body.iter().enumerate() {
        if matches!(
            instruction.op,
            Op::If(_)
                | Op::Else
                | Op::Br(_)
                | Op::BrIf(_)
                | Op::BrTable { .. }
                | Op::Return
                | Op::ReturnCall(_)
                | Op::ReturnCallIndirect { .. }
                | Op::Unreachable
                | Op::Throw(_)
        ) {
            boundaries.insert(index + 1);
        }
    }
    let boundaries = boundaries.into_iter().collect::<Vec<_>>();
    let mut blocks = Vec::with_capacity(boundaries.len().saturating_sub(1));
    let mut instruction_blocks = vec![BlockId(0); instruction_count];
    for range in boundaries.windows(2) {
        if range[0] == range[1] {
            continue;
        }
        let id = BlockId(blocks.len());
        for instruction in range[0]..range[1] {
            instruction_blocks[instruction] = id;
        }
        blocks.push(BasicBlock {
            instructions: range[0]..range[1],
            terminator: Terminator::Unreachable,
            reachable: false,
        });
    }
    let block_at = |instruction: usize| {
        (instruction < instruction_count).then(|| instruction_blocks[instruction])
    };
    for block in &mut blocks {
        let instruction_index = block.instructions.end - 1;
        let op = &function.body[instruction_index].op;
        let fallthrough = block_at(block.instructions.end);
        block.terminator = match op {
            Op::If(_) => {
                let region = layout.opener_regions[instruction_index]
                    .ok_or_else(|| internal("MIR if is missing its control region"))?;
                let region = &layout.regions[region];
                Terminator::Branch {
                    then_block: block_at(instruction_index + 1),
                    else_block: block_at(
                        region.else_at.map_or(region.end + 1, |else_at| else_at + 1),
                    ),
                }
            }
            Op::Else => {
                let node = layout.active_heads[instruction_index]
                    .ok_or_else(|| internal("MIR else is missing its control region"))?;
                let region = layout.active_nodes[node].region;
                Terminator::Jump(block_at(layout.regions[region].end + 1))
            }
            Op::Br(depth) => Terminator::Jump(branch_target(
                layout,
                layout.active_heads[instruction_index],
                *depth,
                &block_at,
            )),
            Op::BrIf(depth) => Terminator::Branch {
                then_block: branch_target(
                    layout,
                    layout.active_heads[instruction_index],
                    *depth,
                    &block_at,
                ),
                else_block: fallthrough,
            },
            Op::BrTable { targets, default } => Terminator::Switch {
                targets: targets
                    .iter()
                    .map(|depth| {
                        branch_target(
                            layout,
                            layout.active_heads[instruction_index],
                            *depth,
                            &block_at,
                        )
                    })
                    .collect(),
                default: branch_target(
                    layout,
                    layout.active_heads[instruction_index],
                    *default,
                    &block_at,
                ),
            },
            Op::Return | Op::ReturnCall(_) | Op::ReturnCallIndirect { .. } => Terminator::Return,
            Op::Unreachable | Op::Throw(_) => Terminator::Unreachable,
            Op::End if instruction_index + 1 == instruction_count => Terminator::Return,
            _ => Terminator::Fallthrough(fallthrough),
        };
    }
    Ok((blocks, instruction_blocks))
}

fn branch_target(
    layout: &RegionLayout,
    mut active: Option<usize>,
    depth: u32,
    block_at: &impl Fn(usize) -> Option<BlockId>,
) -> Option<BlockId> {
    for _ in 0..depth {
        active = active.and_then(|node| layout.active_nodes[node].parent);
    }
    let node = active?;
    let region = &layout.regions[layout.active_nodes[node].region];
    block_at(if region.kind == RegionKind::Loop {
        region.start + 1
    } else {
        region.end + 1
    })
}

fn mark_reachable(mir: &mut FunctionMir) {
    for block in &mut mir.blocks {
        block.reachable = false;
    }
    if mir.blocks.is_empty() {
        return;
    }
    let mut queue = VecDeque::from([BlockId(0)]);
    while let Some(block) = queue.pop_front() {
        if mir.blocks[block.0].reachable {
            continue;
        }
        mir.blocks[block.0].reachable = true;
        queue.extend(mir.blocks[block.0].terminator.successors());
    }
}

fn propagate_constants(mir: &FunctionMir) -> Vec<Option<i32>> {
    let mut constants = vec![None; mir.values.len()];
    for (index, value) in mir.values.iter().enumerate() {
        constants[index] = match value.def {
            ValueDef::I32Const(value) => Some(value),
            ValueDef::Unary(op, operand) => {
                constants[operand.0].and_then(|value| fold_i32_unary(op, value))
            }
            ValueDef::Binary(op, left, right) => constants[left.0].and_then(|left| {
                constants[right.0].and_then(|right| fold_i32_binary(op, left, right))
            }),
            ValueDef::LocalGet {
                alias: Some(value), ..
            } => constants[value.0],
            ValueDef::LocalGet { alias: None, .. } | ValueDef::Load(_, _, _) | ValueDef::Opaque => {
                None
            }
        };
    }
    constants
}

fn plan_constant_branches(
    mir: &FunctionMir,
    function: &Function,
    constants: &[Option<i32>],
    plan: &mut FunctionPlan,
) {
    for (instruction, value) in mir.control_values.iter().enumerate() {
        if !mir.blocks[mir.instruction_blocks[instruction].0].reachable {
            continue;
        }
        let Some(value) = value else {
            continue;
        };
        let Some(value) = constants[value.0] else {
            continue;
        };
        match function.body[instruction].op {
            Op::BrIf(_) => {
                plan.branches[instruction] = Some(if value == 0 {
                    BranchDecision::Never
                } else {
                    BranchDecision::Always
                });
            }
            Op::BrTable { .. } => {
                plan.switches[instruction] = Some(SwitchDecision::Constant(value as u32));
            }
            _ => {}
        }
    }
}

fn plan_switch_canonicalization(
    mir: &FunctionMir,
    function: &Function,
    constants: &[Option<i32>],
    plan: &mut FunctionPlan,
) {
    for (instruction, value) in mir.control_values.iter().enumerate() {
        if !mir.blocks[mir.instruction_blocks[instruction].0].reachable {
            continue;
        }
        let Op::BrTable { ref targets, .. } = function.body[instruction].op else {
            continue;
        };
        if plan.switches[instruction].is_some() || targets.len() > 256 {
            continue;
        }
        let Some(value) = value else {
            continue;
        };
        if let Some((local, right)) = rotated_u8_local(mir, constants, *value) {
            plan.switches[instruction] = Some(SwitchDecision::RotateLocal { local, left: right });
        }
    }
}

fn plan_loop_reciprocal_divisions(mir: &FunctionMir, function: &Function, plan: &mut FunctionPlan) {
    let mut loop_blocks = vec![false; mir.blocks.len()];
    for (block_index, block) in mir.blocks.iter().enumerate() {
        if !block.reachable {
            continue;
        }
        for successor in block
            .terminator
            .successors()
            .filter(|successor| successor.0 <= block_index)
        {
            loop_blocks[successor.0..=block_index].fill(true);
        }
    }

    for (instruction, item) in function.body.iter().enumerate() {
        let block = mir.instruction_blocks[instruction];
        if loop_blocks[block.0] && matches!(item.op, Op::Binary(BinaryOp::I32DivS)) {
            plan.reciprocal_divisions[instruction] = true;
        }
    }
}

fn rotated_u8_local(
    mir: &FunctionMir,
    constants: &[Option<i32>],
    value: ValueId,
) -> Option<(u32, u32)> {
    let (rotated, mask) = binary_with_constant(mir, constants, value, BinaryOp::I32And)?;
    if mask != 255 {
        return None;
    }
    let (left, right) = binary_operands(mir, rotated, BinaryOp::I32Or)?;
    rotated_u8_halves(mir, constants, left, right)
        .or_else(|| rotated_u8_halves(mir, constants, right, left))
}

fn rotated_u8_halves(
    mir: &FunctionMir,
    constants: &[Option<i32>],
    shifted_left: ValueId,
    shifted_right: ValueId,
) -> Option<(u32, u32)> {
    let (base, left) = binary_with_constant(mir, constants, shifted_left, BinaryOp::I32Shl)?;
    let (masked, right) = binary_with_constant(mir, constants, shifted_right, BinaryOp::I32ShrU)?;
    let (other_base, mask) = binary_with_constant(mir, constants, masked, BinaryOp::I32And)?;
    let left = left as u32;
    let right = right as u32;
    if left + right != 8 || right == 0 || right >= 8 {
        return None;
    }
    if mask as u32 != ((255u32 << right) & 255) {
        return None;
    }
    let local = local_value(mir, base)?;
    let other_local = local_value(mir, other_base)?;
    (other_local == local).then_some((local.0, right))
}

fn local_value(mir: &FunctionMir, value: ValueId) -> Option<(u32, u32)> {
    match mir.values[value.0].def {
        ValueDef::LocalGet { local, version, .. } => Some((local, version)),
        _ => None,
    }
}

fn binary_operands(
    mir: &FunctionMir,
    value: ValueId,
    expected: BinaryOp,
) -> Option<(ValueId, ValueId)> {
    match mir.values[value.0].def {
        ValueDef::Binary(op, left, right) if op == expected => Some((left, right)),
        _ => None,
    }
}

fn binary_with_constant(
    mir: &FunctionMir,
    constants: &[Option<i32>],
    value: ValueId,
    expected: BinaryOp,
) -> Option<(ValueId, i32)> {
    let (left, right) = binary_operands(mir, value, expected)?;
    if let Some(constant) = constants[right.0] {
        Some((left, constant))
    } else if is_commutative(expected) {
        constants[left.0].map(|constant| (right, constant))
    } else {
        None
    }
}

fn is_commutative(op: BinaryOp) -> bool {
    matches!(
        op,
        BinaryOp::I32Add
            | BinaryOp::I32Mul
            | BinaryOp::I32And
            | BinaryOp::I32Or
            | BinaryOp::I32Xor
            | BinaryOp::I32Eq
            | BinaryOp::I32Ne
    )
}

fn fold_i32_unary(op: UnaryOp, value: i32) -> Option<i32> {
    Some(match op {
        UnaryOp::I32Eqz => (value == 0) as i32,
        UnaryOp::I32Clz => value.leading_zeros() as i32,
        UnaryOp::I32Ctz => value.trailing_zeros() as i32,
        UnaryOp::I32Popcnt => value.count_ones() as i32,
        UnaryOp::I32Extend8S => value as i8 as i32,
        UnaryOp::I32Extend16S => value as i16 as i32,
        _ => return None,
    })
}

fn fold_i32_binary(op: BinaryOp, left: i32, right: i32) -> Option<i32> {
    let shift = (right as u32) & 31;
    Some(match op {
        BinaryOp::I32Eq => (left == right) as i32,
        BinaryOp::I32Ne => (left != right) as i32,
        BinaryOp::I32LtS => (left < right) as i32,
        BinaryOp::I32LtU => ((left as u32) < (right as u32)) as i32,
        BinaryOp::I32GtS => (left > right) as i32,
        BinaryOp::I32GtU => ((left as u32) > (right as u32)) as i32,
        BinaryOp::I32LeS => (left <= right) as i32,
        BinaryOp::I32LeU => ((left as u32) <= (right as u32)) as i32,
        BinaryOp::I32GeS => (left >= right) as i32,
        BinaryOp::I32GeU => ((left as u32) >= (right as u32)) as i32,
        BinaryOp::I32Add => left.wrapping_add(right),
        BinaryOp::I32Sub => left.wrapping_sub(right),
        BinaryOp::I32Mul => left.wrapping_mul(right),
        BinaryOp::I32DivS if right != 0 && !(left == i32::MIN && right == -1) => left / right,
        BinaryOp::I32DivU if right != 0 => ((left as u32) / (right as u32)) as i32,
        BinaryOp::I32RemS if right != 0 => left.wrapping_rem(right),
        BinaryOp::I32RemU if right != 0 => ((left as u32) % (right as u32)) as i32,
        BinaryOp::I32And => left & right,
        BinaryOp::I32Or => left | right,
        BinaryOp::I32Xor => left ^ right,
        BinaryOp::I32Shl => left.wrapping_shl(shift),
        BinaryOp::I32ShrS => left.wrapping_shr(shift),
        BinaryOp::I32ShrU => ((left as u32) >> shift) as i32,
        BinaryOp::I32Rotl => left.rotate_left(shift),
        BinaryOp::I32Rotr => left.rotate_right(shift),
        _ => return None,
    })
}

fn pop_or_opaque(
    mir: &mut FunctionMir,
    stack: &mut Vec<ValueId>,
    ty: ValType,
    source: usize,
) -> ValueId {
    stack.pop().unwrap_or_else(|| mir.opaque(ty, source))
}

fn pop_many(stack: &mut Vec<ValueId>, count: usize) {
    for _ in 0..count {
        stack.pop();
    }
}

fn apply_call(
    mir: &mut FunctionMir,
    ty: &crate::ir::FuncType,
    stack: &mut Vec<ValueId>,
    source: usize,
) {
    pop_many(stack, ty.params.len());
    for &result in &ty.results {
        let value = mir.opaque(result, source);
        stack.push(value);
    }
}

fn apply_simd(mir: &mut FunctionMir, op: &SimdOp, stack: &mut Vec<ValueId>, source: usize) {
    use SimdOp::*;
    let (pops, result) = match op {
        Const(_) => (0, Some(ValType::V128)),
        I32x4Splat | F32x4Splat => (1, Some(ValType::V128)),
        I32x4Add | I32x4Sub | I32x4Mul | F32x4Add | F32x4Sub | F32x4Mul | F32x4Div
        | I8x16Shuffle(_) | V128And | V128Or | V128Xor => (2, Some(ValType::V128)),
        I32x4Shl => (2, Some(ValType::V128)),
        I32x4Extract(_) => (1, Some(ValType::I32)),
        F32x4Extract(_) => (1, Some(ValType::F32)),
        I32x4Replace(_) | F32x4Replace(_) => (2, Some(ValType::V128)),
        F32x4Abs | F32x4Neg | F32x4Sqrt | V128Not => (1, Some(ValType::V128)),
        V128Bitselect => (3, Some(ValType::V128)),
        V128AnyTrue => (1, Some(ValType::I32)),
        V128Load(_) => (1, Some(ValType::V128)),
        V128Store(_) => (2, None),
        Other(_) => {
            stack.clear();
            return;
        }
    };
    pop_many(stack, pops);
    if let Some(ty) = result {
        let value = mir.opaque(ty, source);
        stack.push(value);
    }
}

fn module_function_type(
    module: &Module,
    function_index: usize,
) -> Result<&crate::ir::FuncType, CompileError> {
    let type_index = *module
        .function_type_indices
        .get(function_index)
        .ok_or_else(|| {
            internal(format!(
                "MIR function index {function_index} is out of range"
            ))
        })?;
    module
        .types
        .get(type_index as usize)
        .ok_or_else(|| internal(format!("MIR type index {type_index} is out of range")))
}

fn memory_index_type(module: &Module, memory: u32) -> Result<ValType, CompileError> {
    module
        .memories
        .get(memory as usize)
        .map(|memory| {
            if memory.memory64 {
                ValType::I64
            } else {
                ValType::I32
            }
        })
        .ok_or_else(|| internal(format!("MIR memory index {memory} is out of range")))
}

fn table_index_type(module: &Module, table: u32) -> Result<ValType, CompileError> {
    module
        .tables
        .get(table as usize)
        .map(|table| {
            if table.table64 {
                ValType::I64
            } else {
                ValType::I32
            }
        })
        .ok_or_else(|| internal(format!("MIR table index {table} is out of range")))
}

fn load_natural_alignment(op: LoadOp) -> u8 {
    match op {
        LoadOp::I32_8S | LoadOp::I32_8U | LoadOp::I64_8S | LoadOp::I64_8U => 0,
        LoadOp::I32_16S | LoadOp::I32_16U | LoadOp::I64_16S | LoadOp::I64_16U => 1,
        LoadOp::I32 | LoadOp::F32 | LoadOp::I64_32S | LoadOp::I64_32U => 2,
        LoadOp::I64 | LoadOp::F64 => 3,
    }
}

fn load_result_type(op: LoadOp) -> ValType {
    match op {
        LoadOp::I32 | LoadOp::I32_8S | LoadOp::I32_8U | LoadOp::I32_16S | LoadOp::I32_16U => {
            ValType::I32
        }
        LoadOp::I64
        | LoadOp::I64_8S
        | LoadOp::I64_8U
        | LoadOp::I64_16S
        | LoadOp::I64_16U
        | LoadOp::I64_32S
        | LoadOp::I64_32U => ValType::I64,
        LoadOp::F32 => ValType::F32,
        LoadOp::F64 => ValType::F64,
    }
}

fn unary_operand_type(op: UnaryOp) -> ValType {
    use UnaryOp::*;
    match op {
        I32Eqz | I32Clz | I32Ctz | I32Popcnt | I64ExtendI32S | I64ExtendI32U | F32ConvertI32S
        | F32ConvertI32U | F64ConvertI32S | F64ConvertI32U | F32ReinterpretI32 | I32Extend8S
        | I32Extend16S => ValType::I32,
        I64Eqz | I64Clz | I64Ctz | I64Popcnt | I32WrapI64 | F32ConvertI64S | F32ConvertI64U
        | F64ConvertI64S | F64ConvertI64U | F64ReinterpretI64 | I64Extend8S | I64Extend16S
        | I64Extend32S => ValType::I64,
        F32Abs | F32Neg | F32Ceil | F32Floor | F32Trunc | F32Nearest | F32Sqrt | I32TruncF32S
        | I32TruncF32U | I64TruncF32S | I64TruncF32U | F64PromoteF32 | I32ReinterpretF32
        | I32TruncSatF32S | I32TruncSatF32U | I64TruncSatF32S | I64TruncSatF32U => ValType::F32,
        F64Abs | F64Neg | F64Ceil | F64Floor | F64Trunc | F64Nearest | F64Sqrt | I32TruncF64S
        | I32TruncF64U | I64TruncF64S | I64TruncF64U | F32DemoteF64 | I64ReinterpretF64
        | I32TruncSatF64S | I32TruncSatF64U | I64TruncSatF64S | I64TruncSatF64U => ValType::F64,
    }
}

fn unary_result_type(op: UnaryOp) -> ValType {
    use UnaryOp::*;
    match op {
        I32Eqz | I64Eqz | I32Clz | I32Ctz | I32Popcnt | I32WrapI64 | I32TruncF32S
        | I32TruncF32U | I32TruncF64S | I32TruncF64U | I32ReinterpretF32 | I32Extend8S
        | I32Extend16S | I32TruncSatF32S | I32TruncSatF32U | I32TruncSatF64S | I32TruncSatF64U => {
            ValType::I32
        }
        I64Clz | I64Ctz | I64Popcnt | I64ExtendI32S | I64ExtendI32U | I64TruncF32S
        | I64TruncF32U | I64TruncF64S | I64TruncF64U | I64ReinterpretF64 | I64Extend8S
        | I64Extend16S | I64Extend32S | I64TruncSatF32S | I64TruncSatF32U | I64TruncSatF64S
        | I64TruncSatF64U => ValType::I64,
        F32Abs | F32Neg | F32Ceil | F32Floor | F32Trunc | F32Nearest | F32Sqrt | F32ConvertI32S
        | F32ConvertI32U | F32ConvertI64S | F32ConvertI64U | F32DemoteF64 | F32ReinterpretI32 => {
            ValType::F32
        }
        F64Abs | F64Neg | F64Ceil | F64Floor | F64Trunc | F64Nearest | F64Sqrt | F64ConvertI32S
        | F64ConvertI32U | F64ConvertI64S | F64ConvertI64U | F64PromoteF32 | F64ReinterpretI64 => {
            ValType::F64
        }
    }
}

fn binary_operand_type(op: BinaryOp) -> ValType {
    use BinaryOp::*;
    match op {
        I32Eq | I32Ne | I32LtS | I32LtU | I32GtS | I32GtU | I32LeS | I32LeU | I32GeS | I32GeU
        | I32Add | I32Sub | I32Mul | I32DivS | I32DivU | I32RemS | I32RemU | I32And | I32Or
        | I32Xor | I32Shl | I32ShrS | I32ShrU | I32Rotl | I32Rotr => ValType::I32,
        I64Eq | I64Ne | I64LtS | I64LtU | I64GtS | I64GtU | I64LeS | I64LeU | I64GeS | I64GeU
        | I64Add | I64Sub | I64Mul | I64DivS | I64DivU | I64RemS | I64RemU | I64And | I64Or
        | I64Xor | I64Shl | I64ShrS | I64ShrU | I64Rotl | I64Rotr => ValType::I64,
        F32Eq | F32Ne | F32Lt | F32Gt | F32Le | F32Ge | F32Add | F32Sub | F32Mul | F32Div
        | F32Min | F32Max | F32Copysign => ValType::F32,
        F64Eq | F64Ne | F64Lt | F64Gt | F64Le | F64Ge | F64Add | F64Sub | F64Mul | F64Div
        | F64Min | F64Max | F64Copysign => ValType::F64,
    }
}

fn binary_result_type(op: BinaryOp) -> ValType {
    use BinaryOp::*;
    match op {
        I32Eq | I32Ne | I32LtS | I32LtU | I32GtS | I32GtU | I32LeS | I32LeU | I32GeS | I32GeU
        | I64Eq | I64Ne | I64LtS | I64LtU | I64GtS | I64GtU | I64LeS | I64LeU | I64GeS | I64GeU
        | F32Eq | F32Ne | F32Lt | F32Gt | F32Le | F32Ge | F64Eq | F64Ne | F64Lt | F64Gt | F64Le
        | F64Ge => ValType::I32,
        _ => binary_operand_type(op),
    }
}

fn internal(message: impl Into<String>) -> CompileError {
    CompileError::new(ErrorKind::Internal, message)
}
