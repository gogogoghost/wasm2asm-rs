(module
      (func (export "equal") (param i64 i64) (result i32)
        local.get 0
        local.get 1
        i64.eq
        if (result i32)
          i32.const 1
        else
          i32.const 0
        end))
