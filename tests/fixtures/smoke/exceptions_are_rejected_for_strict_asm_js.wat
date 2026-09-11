(module
      (type $payload (func (param i32)))
      (tag $error (type $payload))
      (func (export "throwing") (param i32)
        local.get 0 throw $error))
