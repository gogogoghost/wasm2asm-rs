use crate::common::{DifferentialCase, assert_differential};
use wasm2asm::CompileOptions;

#[test]
fn word_division_boundaries_match_native_wasm() {
    let wat = include_str!("../fixtures/differential/word_division_boundaries.wat");
    let expression = r#"(()=>{
        function call(name, a, b) { return CALL; }
        function bits(pair) {
            return BigInt.asUintN(64, BigInt(pair[0] >>> 0) |
                (BigInt(pair[1] >>> 0) << 32n));
        }
        var divisors = [1n,2n,3n,65535n,65536n,65537n,
            0x7fffffffn,0x80000000n,0x80000001n,0xffffffffn,
            0x100000000n,0x100000001n,0x7fffffffffffffffn,
            0x8000000000000000n,0xffffffffffffffffn];
        var values = [0n,1n,2n,3n,65535n,65536n,65537n,
            0x7fffffffn,0x80000000n,0xffffffffn,0x100000000n,
            0x100000001n,0x1fffffffffffffn,0x20000000000000n,
            0x20000000000001n,0x7ffffffffffffffen,0x7fffffffffffffffn,
            0x8000000000000000n,0x8000000000000001n,
            0xfffffffffffffffen,0xffffffffffffffffn];
        // Multiples and immediate neighbors stress quotient digit boundaries.
        divisors.forEach(d => {
            [1n,65535n,65536n,65537n,0xffffffffn].forEach(q => {
                [-1n,0n,1n].forEach(delta => {
                    var a = d*q+delta;
                    if (a >= 0n && a <= 0xffffffffffffffffn) values.push(a);
                });
            });
        });
        var seed = 0x123456789abcdefn;
        for (var i=0;i<96;i++) {
            seed = BigInt.asUintN(64, seed*6364136223846793005n+1442695040888963407n);
            values.push(seed);
        }
        var results = [];
        function check(a,b,signed) {
            a = signed ? BigInt.asIntN(64,a) : BigInt.asUintN(64,a);
            b = signed ? BigInt.asIntN(64,b) : BigInt.asUintN(64,b);
            if (!b) return;
            var suffix = signed ? '_s' : '_u';
            var rpair = call('rem'+suffix,a,b), r = bits(rpair);
            if (signed) r = BigInt.asIntN(64,r);
            // MIN/-1 traps only for division; its remainder is still zero.
            if (signed && a === -0x8000000000000000n && b === -1n) {
                if (r !== 0n) throw new Error('overflow remainder');
                results.push(rpair);
                return;
            }
            var qpair = call('div'+suffix,a,b), q = bits(qpair);
            if (signed) q = BigInt.asIntN(64,q);
            if (q*b+r !== a || (r < 0n ? -r : r) >= (b < 0n ? -b : b) ||
                (r !== 0n && ((r < 0n) !== (a < 0n)))) {
                throw new Error('division identity');
            }
            results.push(qpair,rpair);
        }
        values.forEach(a => divisors.forEach(b => {
            check(a,b,false);
            check(a,b,true);
            check(-a,b,true);
            check(a,-b,true);
            check(-a,-b,true);
        }));
        ['div_u','rem_u','div_s','rem_s'].forEach(name => {
            [0n,1n,-1n,0x8000000000000000n].forEach(a => {
                results.push(trapped(() => call(name,a,0n)));
            });
        });
        results.push(trapped(() => call('div_s',-0x8000000000000000n,-1n)));
        // Follow traps with successful calls to exercise the shared high word.
        results.push(call('rem_s',-0x8000000000000000n,-1n));
        results.push(call('div_u',0xffffffffffffffffn,1n));
        results.push(call('rem_s',-0x20000000000001n,65537n));
        return results;
    })()"#;
    let native_expression = expression.replace("CALL", "i64Pair(m[name](a,b))");
    let asm_expression = expression.replace(
        "CALL",
        "(()=>{var low=m[name](...i64Pair(a),...i64Pair(b));return [low,m.getTempRet0()];})()",
    );
    for preserve_traps in [true, false] {
        assert_differential(DifferentialCase {
            name: "word division boundaries",
            wat,
            native_expression: &native_expression,
            asm_expression: &asm_expression,
            native_imports: "{}",
            asm_imports: "{}",
            options: CompileOptions {
                preserve_traps,
                ..CompileOptions::default()
            },
        });
    }
}
