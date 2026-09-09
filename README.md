# wasm2asm-rs

独立的 WebAssembly 到严格 asm.js 转换器。生成文件包含 asm.js 核心和完整实例化胶水，运行时不再需要原始 `.wasm` 文件，适合在现代项目中作为旧浏览器 fallback。

## 构建

```bash
cargo build --release
```

可执行文件位于 `target/release/wasm2asm`。

## 生成产物

默认生成 ES Module，适合 Vite、Rollup、webpack 等现代构建工具：

```bash
wasm2asm input.wasm -o input.asm.mjs
```

```js
import instantiate from "./input.asm.mjs";

const exports = instantiate({
  env: {
    // WebAssembly imports
  },
});

console.log(exports.main());
```

产物导出同一个工厂的 named export 和 default export：

```js
import instantiate, { instantiate as createInstance } from "./input.asm.mjs";
```

## 直接用于旧浏览器

UMD 格式同时支持浏览器全局变量和 CommonJS：

```bash
wasm2asm input.wasm \
  --format=umd \
  --global-name=LegacyApp \
  -o input.asm.js
```

浏览器：

```html
<script src="input.asm.js"></script>
<script>
  var exports = LegacyApp.instantiate({ env: {} });
  exports.main();
</script>
```

CommonJS：

```js
const { instantiate } = require("./input.asm.js");
const exports = instantiate({ env: {} });
```

`--format=bare` 输出未包装的 `asmModule` 和 `instantiate`，用于自定义打包或 validator 测试。

## Wasm fallback

现代项目可以保留原生 Wasm，同时把 asm.js 作为兼容路径：

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

生成的 `asmModule` 已针对严格 validator 排版。若构建工具会重写函数体，应将产物作为静态资源复制，或排除在二次压缩和语法变换之外；产物本身已经是紧凑格式。

## 转换选项

```text
--format=esm|umd|bare
--global-name=NAME
--enable-lowering=simd
--enable-lowering=references
--enable-all-lowerings
--fast
--max-*=VALUE
```

SIMD 和 typed function references 属于高开销转换，默认关闭。`--fast` 会放宽部分 WebAssembly trap 语义，不建议用于需要精确兼容的构建。

## 兼容范围

支持标量 Wasm、i64 low/high ABI、multivalue exports、mutable globals、bulk memory、内部函数表、memory64、table64、多内存，以及可选的 SIMD 和 typed function-reference lowering。

threads/shared memory、Wasm GC、Wasm exceptions、递归或间接 tail calls，以及无法通过 asm.js host ABI 表达的边界类型会返回明确诊断，不会生成伪装成合法 asm.js 的降级文件。

## 验证

```bash
cargo test
cargo clippy --all-targets -- -D warnings
```

回归测试会执行生成的 ESM、UMD、CommonJS 和 bare 产物，并检查 V8 是否接受其中的严格 asm.js 核心。
