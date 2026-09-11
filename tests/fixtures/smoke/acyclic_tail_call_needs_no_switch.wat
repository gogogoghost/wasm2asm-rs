(module
      (func $increment (param i32) (result i32) local.get 0 i32.const 1 i32.add)
      (func (export "call") (param i32) (result i32) local.get 0 return_call $increment))
