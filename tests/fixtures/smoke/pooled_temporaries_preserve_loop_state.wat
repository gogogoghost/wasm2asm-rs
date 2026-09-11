(module
      (func (export "sum") (param $n i32) (result i32)
        (local $i i32) (local $acc i32)
        local.get $n
        local.set $i
        i32.const 0
        local.set $acc
        block $exit
          loop $loop
            local.get $i
            i32.eqz
            br_if $exit
            local.get $acc
            local.get $i
            i32.const 3
            i32.mul
            i32.add
            local.set $acc
            local.get $i
            i32.const 1
            i32.sub
            local.set $i
            br $loop
          end
        end
        local.get $acc))
