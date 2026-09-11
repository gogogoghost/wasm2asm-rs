(module
      (type $unary (func (param i32) (result i32)))
      (func $inc (type $unary) (param i32) (result i32)
        local.get 0 i32.const 1 i32.add)
      (elem declare func $inc)
      (func (export "apply") (param i32) (result i32)
        local.get 0 ref.func $inc call_ref $unary))
