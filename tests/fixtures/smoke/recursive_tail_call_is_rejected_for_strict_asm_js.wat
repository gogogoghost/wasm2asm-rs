(module
      (func $sum (export "sum") (param $n i32) (param $acc i32) (result i32)
        local.get $n
        if (result i32)
          local.get $n
          i32.const 1
          i32.sub
          local.get $acc
          local.get $n
          i32.add
          return_call $sum
        else
          local.get $acc
        end))
