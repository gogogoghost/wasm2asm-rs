(module
      (import "env" "memory" (memory 1 2))
      (func (export "roundTrip") (param i32) (result i32)
        i32.const 16
        local.get 0
        i32.store
        i32.const 16
        i32.load))
