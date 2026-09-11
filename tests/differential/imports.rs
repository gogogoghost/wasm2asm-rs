use crate::common::{DifferentialCase, assert_differential};
use wasm2asm::{CompileOptions, Lowerings};

#[test]
fn imports_start_and_mutable_globals_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/imports_start_and_mutable_globals.wat");
    let imports = r#"{env:{base:40,add:function(a,b){return (a+b)|0;}}}"#;
    assert_differential(DifferentialCase {
        name: "imports start and mutable globals",
        wat,
        native_expression: "(()=>{var before=m.state.value;m.state.value=7;return [before,m.run(5),m.state.value]})()",
        asm_expression: "(()=>{var before=m.state.value;m.state.value=7;return [before,m.run(5),m.state.value]})()",
        native_imports: imports,
        asm_imports: imports,
        options: Default::default(),
    });
}
#[test]
fn i64_imports_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/i64_imports.wat");
    assert_differential(DifferentialCase {
        name: "i64 imports",
        wat,
        native_expression: "[i64Pair(m.readGlobal()),i64Pair(m.callImport())]",
        asm_expression: "(()=>{var a=m.readGlobal(),ah=m.getTempRet0(),b=m.callImport(),bh=m.getTempRet0();return [[a,ah],[b,bh]]})()",
        native_imports: "{env:{value:4294967298n,next:function(){return 8589934595n;}}}",
        asm_imports: "{env:{value:{low:2,high:1},next:function(){return 3;},getTempRet0:function(){return 2;}}}",
        options: Default::default(),
    });
}

#[test]
fn globals_and_typed_selects_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/globals_and_typed_selects.wat");
    let options = CompileOptions {
        lowerings: Lowerings::SIMD,
        ..CompileOptions::default()
    };
    assert_differential(DifferentialCase {
        name: "globals and typed selects",
        wat,
        native_expression: "(()=>{var a=m.selects(0),b=m.selects(1);return [m.integer.value,i64Pair(m.wide.value),m.single.value,m.double.value,[a[0],a[1],i64Pair(a[2]),a[3],a[4]],[b[0],b[1],i64Pair(b[2]),b[3],b[4]]]})()",
        asm_expression: "(()=>{return [m.integer.value,[m.wide.low,m.wide.high],m.single.value,m.double.value,m.selects(0),m.selects(1)]})()",
        native_imports: "{env:{base:7,wide:4294967298n,single:1.5,double:-2.25}}",
        asm_imports: "{env:{base:7,wide:{low:2,high:1},single:1.5,double:-2.25}}",
        options,
    });
}
