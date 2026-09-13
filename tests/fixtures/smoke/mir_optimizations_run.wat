(module
  (func (export "classify") (param $value i32) (result i32) (local $selector i32)
    (block $default
      (block $case2
        (block $case1
          (block $case0
            local.get $value
            local.tee $selector
            i32.const 7
            i32.shl
            local.get $selector
            i32.const 254
            i32.and
            i32.const 1
            i32.shr_u
            i32.or
            i32.const 255
            i32.and
            br_table $case0 $case1 $case2 $default)
          i32.const 10
          return)
        i32.const 20
        return)
      i32.const 30
      return)
    i32.const 40)

  (func (export "never") (result i32)
    (block $taken
      i32.const 0
      br_if $taken
      i32.const 42
      return)
    i32.const 0)

  (func (export "always") (result i32)
    (block $taken
      i32.const 20
      i32.const 22
      i32.add
      br_if $taken
      i32.const 0
      return)
    i32.const 42))
