(module
      (func (export "clz") (param i64) (result i32)
        local.get 0 i64.clz i32.wrap_i64)
      (func (export "ctz") (param i64) (result i32)
        local.get 0 i64.ctz i32.wrap_i64))
