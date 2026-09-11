(module
      (func $inc (param i32) (result i32)
        local.get 0 i32.const 1 i32.add)
      (func (export "call") (param i32) (result i32)
        local.get 0 call $inc)
      (func (export "sum") (param $n i32) (result i32)
        (local $total i32)
        block $exit
          loop $again
            local.get $n
            i32.eqz
            br_if $exit
            local.get $total
            local.get $n
            i32.add
            local.set $total
            local.get $n
            i32.const 1
            i32.sub
            local.tee $n
            drop
            br $again
          end
        end
        local.get $total)
      (func (export "choose") (param i32) (result i32)
        block $default
          block $one
            block $zero
              local.get 0
              br_table $zero $one $default
            end
            i32.const 10
            return
          end
          i32.const 20
          return
        end
        i32.const 30)
      (func (export "branchValue") (param i32) (result i32)
        block (result i32)
          i32.const 7
          local.get 0
          br_if 0
          drop
          i32.const 9
        end)
      (func (export "conditional") (param i32) (result i32)
        local.get 0
        if (result i32)
          i32.const 11
        else
          i32.const 22
        end))
