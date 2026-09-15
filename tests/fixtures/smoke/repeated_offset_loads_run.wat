(module
  (memory 512 512)
  (data (i32.const 0) "\01\02\03\04\05\06\07\08\09\0a\0b\0c\0d\0e\0f\10")
  (func (export "sum") (param $address i32) (result i32)
    local.get $address
    i32.load offset=1
    local.get $address
    i32.load offset=1
    i32.add
    local.get $address
    i32.load offset=1
    i32.add
    local.get $address
    i32.load offset=1
    i32.add
    local.get $address
    i32.load offset=1
    i32.add
    local.get $address
    i32.load offset=1
    i32.add
    local.get $address
    i32.load offset=1
    i32.add
    local.get $address
    i32.load offset=1
    i32.add
    local.get $address
    i32.load offset=1
    i32.add
    local.get $address
    i32.load offset=1
    i32.add
    local.get $address
    i32.load offset=1
    i32.add
    local.get $address
    i32.load offset=1
    i32.add
    local.get $address
    i32.load offset=1
    i32.add
    local.get $address
    i32.load offset=1
    i32.add
    local.get $address
    i32.load offset=1
    i32.add
    local.get $address
    i32.load offset=1
    i32.add)

  (func (export "byte_round_trip") (param $address i32) (param $value i32) (result i32)
    local.get $address
    local.get $value
    i32.store8 offset=2
    local.get $address
    i32.load8_u offset=2)
)
