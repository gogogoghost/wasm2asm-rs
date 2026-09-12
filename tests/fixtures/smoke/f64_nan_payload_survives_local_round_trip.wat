(module
  (memory 1)
  (data (i32.const 0) "\0d\01\00\00\00\00\f8\7f")
  (func (export "bits") (result i64)
    (local $value f64)
    i32.const 0
    f64.load
    local.set $value
    local.get $value
    i64.reinterpret_f64)
  (func (export "selected_bits") (param $condition i32) (result i64)
    (local $value f64)
    i32.const 0
    f64.load
    local.set $value
    local.get $value
    f64.const 0
    local.get $condition
    select
    i64.reinterpret_f64))
