use super::*;
use wasmparser::{AbstractHeapType, BlockType, HeapType, RefType, UnpackedIndex, ValType as W};

#[test]
fn value_types_and_block_signatures_cover_every_shape() {
    for ty in [ValType::I32, ValType::I64, ValType::F32, ValType::F64] {
        assert!(ty.is_scalar());
    }
    assert!(!ValType::V128.is_scalar());
    assert!(!ValType::FuncRef(None).is_scalar());

    let types = vec![FuncType {
        params: vec![ValType::I32, ValType::F64],
        results: vec![ValType::I64],
    }];
    let empty = BlockSig::from_block_type(BlockType::Empty, &types).unwrap();
    assert!(empty.params.is_empty() && empty.results.is_empty());
    let scalar = BlockSig::from_block_type(BlockType::Type(W::F32), &types).unwrap();
    assert!(matches!(scalar.results.as_slice(), [ValType::F32]));
    let indexed = BlockSig::from_block_type(BlockType::FuncType(0), &types).unwrap();
    assert_eq!(indexed.params.len(), 2);
    assert!(matches!(indexed.results.as_slice(), [ValType::I64]));
    assert!(BlockSig::from_block_type(BlockType::FuncType(1), &types).is_none());
}

#[test]
fn wasm_value_type_conversion_covers_reference_kinds() {
    assert_eq!(convert_val_type(W::I32), Some(ValType::I32));
    assert_eq!(convert_val_type(W::I64), Some(ValType::I64));
    assert_eq!(convert_val_type(W::F32), Some(ValType::F32));
    assert_eq!(convert_val_type(W::F64), Some(ValType::F64));
    assert_eq!(convert_val_type(W::V128), Some(ValType::V128));
    assert_eq!(
        convert_val_type(W::Ref(RefType::FUNCREF)),
        Some(ValType::FuncRef(None))
    );
    assert_eq!(
        convert_val_type(W::Ref(RefType::NULLFUNCREF)),
        Some(ValType::FuncRef(None))
    );

    let concrete = RefType::new(true, HeapType::Concrete(UnpackedIndex::Module(7))).unwrap();
    assert_eq!(
        convert_val_type(W::Ref(concrete)),
        Some(ValType::FuncRef(Some(7)))
    );
    let exact = RefType::new(false, HeapType::Exact(UnpackedIndex::Module(9))).unwrap();
    assert_eq!(
        convert_val_type(W::Ref(exact)),
        Some(ValType::FuncRef(Some(9)))
    );
    let extern_ref = RefType::new(
        true,
        HeapType::Abstract {
            shared: false,
            ty: AbstractHeapType::Extern,
        },
    )
    .unwrap();
    assert_eq!(convert_val_type(W::Ref(extern_ref)), None);
}
