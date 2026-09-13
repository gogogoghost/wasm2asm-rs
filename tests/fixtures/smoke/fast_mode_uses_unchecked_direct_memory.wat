(module
  (import "env" "memory" (memory 512 512))
  (func (export "outOfBounds") (result i32)
    i32.const 33554432
    i32.load))
