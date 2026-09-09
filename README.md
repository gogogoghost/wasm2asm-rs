# wasm2asm-rs

A standalone WebAssembly-to-asm.js ahead-of-time converter written in Rust.

`wasm2asm` produces a strictly validated asm.js core together with the JavaScript glue required to instantiate the module. The generated file is self-contained and does not require the original `.wasm` file at runtime, making it suitable as a fallback for legacy browsers and constrained JavaScript environments without WebAssembly support.

## Features

- Standalone Rust implementation with no Binaryen dependency
- Strict asm.js-compatible code generation
- ES module, UMD/CommonJS, and bare output formats
- Complete import, export, memory, table, global, data-segment, and start-function glue
- Scalar Wasm operations and the JavaScript i64 low/high ABI
- Multiple results, bulk memory, mutable globals, and internal function tables
- Bounded memory64, table64, and multiple-memory lowering
- Optional SIMD and typed function-reference lowering
- Configurable resource limits
- Controlled diagnostics for unsupported WebAssembly features

## Build

```bash
cargo build --release
```

The executable is written to:

```text
target/release/wasm2asm
```

## Quick start

The default output format is an ES module suitable for Vite, Rollup, webpack, and other modern build systems:

```bash
wasm2asm input.wasm -o input.asm.mjs
```

Import and instantiate the generated module:

```js
import instantiate from "./input.asm.mjs";

const exports = instantiate({
  env: {
    // WebAssembly imports
  },
});

console.log(exports.main());
```

The factory is available as both a default and named export:

```js
import instantiate, { instantiate as createInstance } from "./input.asm.mjs";
```

## Output formats

### ES module

ES module output is the default:

```bash
wasm2asm input.wasm --format=esm -o input.asm.mjs
```

It exports:

```js
export { instantiate };
export default instantiate;
```

### UMD and CommonJS

Use UMD output for direct browser scripts or CommonJS projects:

```bash
wasm2asm input.wasm \
  --format=umd \
  --global-name=LegacyApp \
  -o input.asm.js
```

Browser usage:

```html
<script src="input.asm.js"></script>
<script>
  var exports = LegacyApp.instantiate({ env: {} });
  exports.main();
</script>
```

CommonJS usage:

```js
const { instantiate } = require("./input.asm.js");
const exports = instantiate({ env: {} });
```

The default browser global name is `Wasm2AsmModule` and can be changed with `--global-name`.

### Bare output

Bare output exposes unwrapped `asmModule` and `instantiate` declarations for custom packaging or validator testing:

```bash
wasm2asm input.wasm --format=bare -o input.asm.js
```

## Native WebAssembly fallback

A modern application can keep native WebAssembly as its primary implementation and use the generated asm.js module only when WebAssembly is unavailable:

```js
import instantiateAsm from "./app.asm.mjs";

export async function loadApp(imports) {
  if (typeof WebAssembly === "object") {
    const response = await fetch("./app.wasm");
    const result = await WebAssembly.instantiateStreaming(response, imports);
    return result.instance.exports;
  }

  return instantiateAsm(imports);
}
```

For browsers that cannot load ES modules, generate UMD output and include it as a regular script instead.

## Command-line options

```text
--format=esm|umd|bare
--global-name=NAME
--enable-lowering=simd
--enable-lowering=references
--enable-all-lowerings
--fast
--max-*=VALUE
```

Run the following command for the complete option list:

```bash
wasm2asm --help
```

SIMD and typed function references are high-cost lowerings and are disabled by default.

`--fast` disables parts of WebAssembly trap preservation. Do not use it when exact WebAssembly failure semantics are required.

## Compatibility

The supported profile includes:

- i32, i64, f32, and f64 scalar operations
- the JavaScript i64 low/high ABI
- multiple-value exports
- mutable scalar and i64 globals
- active and passive data segments
- bulk-memory operations
- internal nullable function tables and indirect-call signature checks
- bounded memory64 and table64 lowering
- multiple memories
- optional fixed-width SIMD scalarization
- optional typed function-reference lowering

Features without a valid strict asm.js representation are rejected with a diagnostic instead of producing invalid JavaScript. These include:

- threads, atomics, and shared memory
- Wasm GC
- Wasm exceptions
- recursive or indirect tail calls
- imported multivalue callbacks
- imported or exported tables
- v128 or function references crossing the JavaScript host boundary
- unsupported proposal instructions

## Build-system integration

The generated `asmModule` function is deliberately formatted for strict asm.js validators. Do not allow a JavaScript optimizer, transpiler, or minifier to rewrite its function body. If a build tool modifies generated code, copy the output as a static asset or exclude it from further syntax transformations. The generated output is already compact.

## Release binaries

Pushing a tag in `MAJOR.MINOR.PATCH` form, such as `0.0.1`, creates a GitHub Release containing:

```text
wasm2asm-0.0.1-linux-x64.tar.gz
wasm2asm-0.0.1-linux-arm64.tar.gz
```

Tags must not use a `v` prefix.

## Verification

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

The integration tests execute generated ES module, UMD, CommonJS, and bare outputs. The generated asm.js core is also checked against V8's strict asm.js handling.
