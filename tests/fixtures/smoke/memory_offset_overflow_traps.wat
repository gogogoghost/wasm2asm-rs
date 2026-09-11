(module
      (memory 1)
      (func (export "load_offset") (param i32) (result i32)
        local.get 0
        i32.load offset=8))
