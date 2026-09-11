(module
      (func (export "pick") (param i32 i64 i64) (result i64)
        local.get 1
        local.get 2
        local.get 0
        select))
