(module
      (func (export "run")
        (result
          i32 i32 i32 i32 i32
          i32 i32 i32 i32 i32
          i32 i32 i32 i32 i32
          i32 i32 i32 i32 i32
          i32 i32 i32 i32 i32
          i32 i32 i32 i32 i32)
        (i32.eqz (i32.const 0))
        (i32.clz (i32.const 16))
        (i32.ctz (i32.const 16))
        (i32.popcnt (i32.const 240))
        (i32.eq (i32.const 7) (i32.const 7))
        (i32.ne (i32.const 7) (i32.const 8))
        (i32.lt_s (i32.const -1) (i32.const 1))
        (i32.lt_u (i32.const 1) (i32.const -1))
        (i32.gt_s (i32.const 2) (i32.const -2))
        (i32.gt_u (i32.const -1) (i32.const 2))
        (i32.le_s (i32.const -2) (i32.const -2))
        (i32.le_u (i32.const 2) (i32.const 3))
        (i32.ge_s (i32.const 4) (i32.const 3))
        (i32.ge_u (i32.const -1) (i32.const -2))
        (i32.add (i32.const 20) (i32.const 22))
        (i32.sub (i32.const 20) (i32.const 22))
        (i32.mul (i32.const 7) (i32.const 6))
        (i32.div_s (i32.const -21) (i32.const 2))
        (i32.div_u (i32.const -1) (i32.const 2))
        (i32.rem_s (i32.const -21) (i32.const 4))
        (i32.rem_u (i32.const -1) (i32.const 10))
        (i32.and (i32.const 90) (i32.const 60))
        (i32.or (i32.const 80) (i32.const 15))
        (i32.xor (i32.const 85) (i32.const 15))
        (i32.shl (i32.const 3) (i32.const 4))
        (i32.shr_s (i32.const -64) (i32.const 3))
        (i32.shr_u (i32.const -1) (i32.const 28))
        (i32.rotl (i32.const 305419896) (i32.const 8))
        (i32.rotr (i32.const 305419896) (i32.const 8))
        (select (result i32) (i32.const 11) (i32.const 22) (i32.const 1))))
