(module
      (memory (export "memory") 1)
      (data (i32.const 0) "ABC")
      (data $passive "xyz123")
      (func (export "run")
        (memory.init $passive (i32.const 8) (i32.const 0) (i32.const 6))
        (memory.copy (i32.const 20) (i32.const 8) (i32.const 6))
        (memory.fill (i32.const 32) (i32.const 65) (i32.const 5))
        data.drop $passive))
