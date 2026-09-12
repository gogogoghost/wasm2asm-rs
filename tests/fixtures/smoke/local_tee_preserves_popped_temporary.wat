(module
  (memory 1)
  (func (export "snapshot") (result i32)
    (local $address i32)
    i32.const 0
    i32.const 42
    i32.store
    local.get $address
    i32.const 0
    i32.load
    local.tee $address
    i32.add))
