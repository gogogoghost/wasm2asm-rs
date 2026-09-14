(module
  (func $match5 (export "match5") (param $value i32) (result i32)
    (local $mixed i32)
    local.get $value
    i32.const 3
    i32.mul
    i32.const 7
    i32.add
    i32.const 11
    i32.xor
    i32.const 13
    i32.mul
    i32.const 17
    i32.add
    i32.const 19
    i32.xor
    i32.const 23
    i32.mul
    i32.const 29
    i32.add
    i32.const 31
    i32.xor
    local.set $mixed
    local.get $value
    i32.const 5
    i32.eq
    if (result i32)
      local.get $mixed
    else
      local.get $mixed
      i32.const 1
      i32.add
    end)

  (func $match9 (export "match9") (param $value i32) (result i32)
    (local $mixed i32)
    local.get $value
    i32.const 3
    i32.mul
    i32.const 7
    i32.add
    i32.const 11
    i32.xor
    i32.const 13
    i32.mul
    i32.const 17
    i32.add
    i32.const 19
    i32.xor
    i32.const 23
    i32.mul
    i32.const 29
    i32.add
    i32.const 31
    i32.xor
    local.set $mixed
    local.get $value
    i32.const 9
    i32.eq
    if (result i32)
      local.get $mixed
    else
      local.get $mixed
      i32.const 1
      i32.add
    end)

  (func $match13 (export "match13") (param $value i32) (result i32)
    (local $mixed i32)
    local.get $value
    i32.const 3
    i32.mul
    i32.const 7
    i32.add
    i32.const 11
    i32.xor
    i32.const 13
    i32.mul
    i32.const 17
    i32.add
    i32.const 19
    i32.xor
    i32.const 23
    i32.mul
    i32.const 29
    i32.add
    i32.const 31
    i32.xor
    local.set $mixed
    local.get $value
    i32.const 13
    i32.eq
    if (result i32)
      local.get $mixed
    else
      local.get $mixed
      i32.const 1
      i32.add
    end)

  (func $match17 (export "match17") (param $value i32) (result i32)
    (local $mixed i32)
    local.get $value
    i32.const 3
    i32.mul
    i32.const 7
    i32.add
    i32.const 11
    i32.xor
    i32.const 13
    i32.mul
    i32.const 17
    i32.add
    i32.const 19
    i32.xor
    i32.const 23
    i32.mul
    i32.const 29
    i32.add
    i32.const 31
    i32.xor
    local.set $mixed
    local.get $value
    i32.const 17
    i32.eq
    if (result i32)
      local.get $mixed
    else
      local.get $mixed
      i32.const 1
      i32.add
    end))
