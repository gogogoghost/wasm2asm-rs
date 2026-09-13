(module
  (func (export "div_repeat")
    (param $dividend i32)
    (param $divisor i32)
    (param $count i32)
    (result i32)
    (local $result i32)
    (block $done
      (loop $repeat
        local.get $count
        i32.eqz
        br_if $done
        local.get $dividend
        local.get $divisor
        i32.div_s
        local.set $result
        local.get $count
        i32.const 1
        i32.sub
        local.set $count
        br $repeat))
    local.get $result))
