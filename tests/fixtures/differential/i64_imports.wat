(module
      (import "env" "value" (global i64))
      (import "env" "next" (func $next (result i64)))
      (func (export "readGlobal") (result i64) global.get 0)
      (func (export "callImport") (result i64)
        call $next
        i64.const 5
        i64.add))
