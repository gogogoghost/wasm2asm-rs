(module
      (import "env" "base" (global i32))
      (import "env" "add" (func $add (param i32 i32) (result i32)))
      (global $state (export "state") (mut i32) (i32.const 0))
      (func $start
        global.get 0
        i32.const 2
        call $add
        global.set $state)
      (start $start)
      (func (export "run") (param i32) (result i32)
        global.get $state
        local.get 0
        call $add))
