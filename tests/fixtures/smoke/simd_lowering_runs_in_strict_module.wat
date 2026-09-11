(module
      (func (export "lane") (param i32) (result i32)
        local.get 0
        i32x4.splat
        v128.const i32x4 1 2 3 4
        i32x4.add
        i32x4.extract_lane 2))
