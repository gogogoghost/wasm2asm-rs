(module
  (memory 512 1024)
  (func (export "run") (result i32)
    (local $ok i32)
    i32.const 1
    local.set $ok

    i32.const 16
    i32.const 305419896
    i32.store
    local.get $ok
    i32.const 16
    i32.load
    i32.const 305419896
    i32.eq
    i32.and
    local.set $ok

    i32.const 21
    i32.const -1985229329
    i32.store align=1
    local.get $ok
    i32.const 21
    i32.load align=1
    i32.const -1985229329
    i32.eq
    i32.and
    local.set $ok

    i32.const 128
    i64.const 81985529216486895
    i64.store
    local.get $ok
    i32.const 128
    i64.load
    i64.const 81985529216486895
    i64.eq
    i32.and
    local.set $ok

    i32.const 133
    i64.const -81985529216486896
    i64.store align=1
    local.get $ok
    i32.const 133
    i64.load align=1
    i64.const -81985529216486896
    i64.eq
    i32.and
    local.set $ok

    i32.const 160
    f32.const 1.5
    f32.store
    local.get $ok
    i32.const 160
    f32.load
    f32.const 1.5
    f32.eq
    i32.and
    local.set $ok

    i32.const 165
    f32.const -2.25
    f32.store align=1
    local.get $ok
    i32.const 165
    f32.load align=1
    f32.const -2.25
    f32.eq
    i32.and
    local.set $ok

    i32.const 176
    f64.const nan:0x12345
    f64.store
    local.get $ok
    i32.const 176
    f64.load
    i64.reinterpret_f64
    i64.const 0x7ff0000000012345
    i64.eq
    i32.and
    local.set $ok

    i32.const 181
    f64.const -0x0p+0
    f64.store align=1
    local.get $ok
    i32.const 181
    f64.load align=1
    i64.reinterpret_f64
    i64.const 0x8000000000000000
    i64.eq
    i32.and
    local.set $ok

    i32.const 200
    i32.const 42
    i32.store offset=12
    local.get $ok
    i32.const 200
    i32.load offset=12
    i32.const 42
    i32.eq
    i32.and
    local.set $ok

    i32.const 240
    i32.const 90
    i32.const 16
    memory.fill
    i32.const 260
    i32.const 240
    i32.const 16
    memory.copy
    local.get $ok
    i32.const 260
    i32.load8_u
    i32.const 90
    i32.eq
    i32.and
    local.set $ok

    i32.const 33554428
    i32.const 610839776
    i32.store
    local.get $ok
    i32.const 33554428
    i32.load
    i32.const 610839776
    i32.eq
    i32.and)
  (func (export "oob") (result i32)
    i32.const 33554429
    i32.load))
