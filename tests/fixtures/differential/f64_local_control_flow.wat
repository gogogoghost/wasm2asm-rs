(module
  (memory (export "memory") 1)
  ;; Skipping the assignment must keep the incoming parameter's bits.
  (func (export "choose") (param $a f64) (param $b f64) (param $take i32) (result i64)
    (if (local.get $take) (then (local.set $a (local.get $b))))
    (i64.reinterpret_f64 (local.get $a)))
  (func (export "store_choice") (param $a f64) (param $b f64) (param $take i32) (result i64)
    (if (local.get $take) (then (local.set $a (local.get $b))))
    (f64.store (i32.const 0) (local.get $a))
    (i64.load (i32.const 0)))
  ;; The first read is emitted before any assignment to the parameter.
  (func (export "loop_param") (param $a f64) (param $b f64) (param $n i32) (result i64)
    (local $result i64)
    (loop $again
      (local.set $result (i64.xor (local.get $result) (i64.reinterpret_f64 (local.get $a))))
      (local.set $a (local.get $b))
      (local.set $n (i32.sub (local.get $n) (i32.const 1)))
      (br_if $again (i32.gt_s (local.get $n) (i32.const 0))))
    (local.get $result))
  ;; A default-zero local is read before its first write, then revisited over
  ;; a back edge. The subsequent NaN's raw payload must not be canonicalized.
  (func (export "loop_local") (param $bits i64) (param $n i32) (result i64)
    (local $a f64) (local $result i64)
    (loop $again
      (local.set $result (i64.xor (local.get $result) (i64.reinterpret_f64 (local.get $a))))
      (local.set $a (f64.reinterpret_i64 (local.get $bits)))
      (local.set $n (i32.sub (local.get $n) (i32.const 1)))
      (br_if $again (i32.gt_s (local.get $n) (i32.const 0))))
    (local.get $result))
  (func (export "branch_local") (param $bits i64) (param $take i32) (result i64)
    (local $a f64)
    (if (local.get $take)
      (then (local.set $a (f64.reinterpret_i64 (local.get $bits)))))
    (i64.reinterpret_f64 (local.get $a)))
  ;; Assignments on both arms, including select and local.tee.
  (func (export "merge") (param $a f64) (param $b f64) (param $take i32) (result i64)
    (local $value f64)
    (local.set $value
      (if (result f64) (local.get $take)
        (then (local.tee $a (local.get $b)))
        (else (select (local.get $a) (local.get $b) (i32.const 1)))))
    (i64.xor (i64.reinterpret_f64 (local.get $value))
             (i64.reinterpret_f64 (local.get $a)))))
