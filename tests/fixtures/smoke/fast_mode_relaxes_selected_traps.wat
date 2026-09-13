(module
  (import "env" "memory" (memory 512 512))
  (func (export "outOfBounds") (result i32)
    i32.const 33554432
    i32.load)
  (func (export "trustedAlignedHint") (result i32)
    i32.const 1
    i32.load)
  (func (export "explicitUnalignedHint") (result i32)
    i32.const 1
    i32.load align=1)
  (func (export "invalidConversion") (result i32)
    f64.const nan
    i32.trunc_f64_s))
