(module
  (memory $first (export "memory") 1)
  (memory $second 1)
  (data $passive "DATA")
  (func (export "run") (result i32 i64 f32 f64 i32 i32)
    (i32.store $second offset=4 (i32.const 8) (i32.const 305419896))
    (i64.store $second offset=8 (i32.const 16) (i64.const 81985529216486895))
    (f32.store $second offset=4 (i32.const 32) (f32.const 3.5))
    (f64.store $second offset=8 (i32.const 40) (f64.const -9.25))
    (memory.init $second $passive (i32.const 0) (i32.const 0) (i32.const 4))
    (memory.copy $first $second (i32.const 0) (i32.const 0) (i32.const 56))
    data.drop $passive
    (i32.load $second offset=4 (i32.const 8))
    (i64.load $second offset=8 (i32.const 16))
    (f32.load $second offset=4 (i32.const 32))
    (f64.load $second offset=8 (i32.const 40))
    memory.size $first
    memory.size $second))
