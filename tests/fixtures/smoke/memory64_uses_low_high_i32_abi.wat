(module
      (memory i64 1 2)
      (func (export "store") (param i64 i32) local.get 0 local.get 1 i32.store)
      (func (export "load") (param i64) (result i32) local.get 0 i32.load)
      (func (export "size") (result i64) memory.size))
