(module
      (global $g (export "g") (mut i64) (i64.const 4294967298))
      (func (export "low") (result i32) global.get $g i32.wrap_i64))
