//! Constant values and folding (spec §1.3.2, compiler §13.6): Lua 5.1 double
//! semantics, `%` as `a - floor(a/b)*b`, concatenation with Lua's `%.14g`,
//! bitwise operators with WoW's `bit` library semantics (32-bit, signed).

use wowlua_luair::ir::{self, Pos};

#[derive(Clone, Debug, PartialEq)]
pub enum Const {
    Nil,
    Bool(bool),
    Number(f64),
    String(Vec<u8>),
}

impl Const {
    pub fn truthy(&self) -> bool {
        !matches!(self, Const::Nil | Const::Bool(false))
    }

    /// The same constant, numbers bit for bit: unlike `==`, `0` and `-0`
    /// differ (they are emitted differently) and a NaN equals itself.
    pub fn same(&self, other: &Const) -> bool {
        match (self, other) {
            (Const::Number(a), Const::Number(b)) => a.to_bits() == b.to_bits(),
            _ => self == other,
        }
    }

    /// The constant as an expression that reads back exactly.
    pub fn to_expr(&self, pos: Pos) -> ir::Expr {
        use ir::ExprKind as K;
        let kind = match self {
            Const::Nil => K::Nil,
            Const::Bool(true) => K::True,
            Const::Bool(false) => K::False,
            Const::String(s) => K::String(quote(s)),
            Const::Number(n) => return number_expr(*n, pos),
        };
        ir::Expr::new(kind, pos)
    }
}

/// A number literal expression; negative numbers, NaN and infinities become
/// parenthesized expressions (spec §1.3.2).
pub fn number_expr(n: f64, pos: Pos) -> ir::Expr {
    use ir::ExprKind as K;
    let paren = |inner: ir::Expr| {
        ir::Expr::new(
            K::Paren {
                inner: Box::new(inner),
                close: Pos::NONE,
            },
            pos,
        )
    };
    let num = |s: &str| ir::Expr::new(K::Number(s.to_string()), pos);
    if n.is_nan() {
        return paren(binary(ir::BinOp::Div, num("0"), num("0"), pos));
    }
    if n.is_infinite() {
        let one = if n > 0.0 {
            num("1")
        } else {
            ir::Expr::new(
                K::Unary {
                    op: ir::UnOp::Neg,
                    operand: Box::new(num("1")),
                },
                pos,
            )
        };
        return paren(binary(ir::BinOp::Div, one, num("0"), pos));
    }
    if n < 0.0 || (n == 0.0 && n.is_sign_negative()) {
        return paren(ir::Expr::new(
            K::Unary {
                op: ir::UnOp::Neg,
                operand: Box::new(num(&format_number(-n))),
            },
            pos,
        ));
    }
    num(&format_number(n))
}

fn binary(op: ir::BinOp, lhs: ir::Expr, rhs: ir::Expr, pos: Pos) -> ir::Expr {
    ir::Expr::new(
        ir::ExprKind::Binary {
            op,
            op_pos: Pos::NONE,
            lhs: Box::new(lhs),
            rhs: Box::new(rhs),
        },
        pos,
    )
}

/// A non-negative finite number written so that Lua reads it back exactly:
/// integers as integers, others in the shortest round-trip form.
pub fn format_number(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 9_007_199_254_740_992.0 {
        return format!("{}", n as i64);
    }
    let plain = format!("{n}");
    if plain.len() <= 20 {
        plain
    } else {
        format!("{n:e}")
    }
}

/// Lua 5.1's `LUAI_NUMFFORMAT` (`%.14g`), used by `tostring` and `..`.
pub fn format_g14(n: f64) -> String {
    if n.is_nan() {
        return "nan".into();
    }
    if n.is_infinite() {
        return if n > 0.0 { "inf" } else { "-inf" }.into();
    }
    if n == 0.0 {
        return if n.is_sign_negative() { "-0" } else { "0" }.into();
    }
    let sci = format!("{n:.13e}");
    let (mantissa, exp) = sci.split_once('e').expect("scientific format");
    let exp: i32 = exp.parse().expect("exponent");
    if !(-4..14).contains(&exp) {
        let mantissa = trim_zeros(mantissa);
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{mantissa}e{sign}{:02}", exp.abs())
    } else {
        let decimals = (13 - exp).max(0) as usize;
        trim_zeros(&format!("{n:.decimals$}")).to_string()
    }
}

fn trim_zeros(s: &str) -> &str {
    if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.')
    } else {
        s
    }
}

/// A Lua string literal for `bytes`; bytes outside valid UTF-8 are
/// escaped, so the emitted file stays UTF-8.
pub fn quote(bytes: &[u8]) -> Vec<u8> {
    let mut raw = vec![true; bytes.len()];
    let mut at = 0;
    for chunk in bytes.utf8_chunks() {
        at += chunk.valid().len();
        raw[at..at + chunk.invalid().len()].fill(false);
        at += chunk.invalid().len();
    }
    let mut out = vec![b'"'];
    for (i, &c) in bytes.iter().enumerate() {
        match c {
            b'"' => out.extend_from_slice(b"\\\""),
            b'\\' => out.extend_from_slice(b"\\\\"),
            b'\n' => out.extend_from_slice(b"\\n"),
            b'\r' => out.extend_from_slice(b"\\r"),
            b'\t' => out.extend_from_slice(b"\\t"),
            _ if c < 32 || c == 127 || !raw[i] => {
                // `\ddd`: three digits when a digit follows, so it cannot merge.
                let next_digit = bytes.get(i + 1).is_some_and(u8::is_ascii_digit);
                if next_digit {
                    out.extend_from_slice(format!("\\{c:03}").as_bytes());
                } else {
                    out.extend_from_slice(format!("\\{c}").as_bytes());
                }
            }
            _ => out.push(c),
        }
    }
    out.push(b'"');
    out
}

/// WoW's `bit` library (LuaBitOp) argument conversion: round to the nearest
/// integer and wrap to a signed 32-bit value.
fn tobit(n: f64) -> Option<i32> {
    if !n.is_finite() {
        return None;
    }
    let rounded = n.round_ties_even();
    Some(rounded.rem_euclid(4_294_967_296.0) as u32 as i32)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BitOp {
    And,
    Or,
    Xor,
    Shl,
    Shr,
}

pub fn fold_bit(op: BitOp, args: &[f64]) -> Option<f64> {
    let ints: Vec<i32> = args.iter().map(|&a| tobit(a)).collect::<Option<_>>()?;
    let result = match op {
        BitOp::And => ints.iter().fold(-1, |a, &b| a & b),
        BitOp::Or => ints.iter().fold(0, |a, &b| a | b),
        BitOp::Xor => ints.iter().fold(0, |a, &b| a ^ b),
        BitOp::Shl => ((ints[0] as u32) << (ints[1] as u32 & 31)) as i32,
        BitOp::Shr => ((ints[0] as u32) >> (ints[1] as u32 & 31)) as i32,
    };
    Some(f64::from(result))
}

pub fn fold_bnot(a: f64) -> Option<f64> {
    Some(f64::from(!tobit(a)?))
}

/// Lua 5.1 `%`.
pub fn lua_mod(a: f64, b: f64) -> f64 {
    a - (a / b).floor() * b
}

fn concat_part(c: &Const) -> Option<Vec<u8>> {
    match c {
        Const::String(s) => Some(s.clone()),
        Const::Number(n) if n.is_finite() => Some(format_g14(*n).into_bytes()),
        _ => None,
    }
}

/// Folds a binary operation on constants. `None` means "not foldable" (the
/// expression stays as it is): string ordering depends on the locale, and
/// operations that would raise errors at run time are left to run time.
pub fn fold_binary(op: wowlua_syntax::ast::BinOp, a: &Const, b: &Const) -> Option<Const> {
    use wowlua_syntax::ast::BinOp as B;
    let num = |c: &Const| match c {
        Const::Number(n) => Some(*n),
        _ => None,
    };
    Some(match op {
        B::Add | B::Sub | B::Mul | B::Div | B::Mod | B::Pow | B::IDiv => {
            let (x, y) = (num(a)?, num(b)?);
            Const::Number(match op {
                B::Add => x + y,
                B::Sub => x - y,
                B::Mul => x * y,
                B::Div => x / y,
                B::Mod => lua_mod(x, y),
                B::Pow => x.powf(y),
                _ => (x / y).floor(),
            })
        }
        B::Concat => {
            let mut s = concat_part(a)?;
            s.extend(concat_part(b)?);
            Const::String(s)
        }
        B::Eq => Const::Bool(a == b && !matches!(a, Const::Number(n) if n.is_nan())),
        B::Ne => Const::Bool(!(a == b && !matches!(a, Const::Number(n) if n.is_nan()))),
        B::Lt | B::Le | B::Gt | B::Ge => {
            let (x, y) = (num(a)?, num(b)?);
            Const::Bool(match op {
                B::Lt => x < y,
                B::Le => x <= y,
                B::Gt => x > y,
                _ => x >= y,
            })
        }
        B::And => {
            if a.truthy() {
                b.clone()
            } else {
                a.clone()
            }
        }
        B::Or | B::Coalesce => {
            if op == B::Coalesce {
                if *a == Const::Nil {
                    b.clone()
                } else {
                    a.clone()
                }
            } else if a.truthy() {
                a.clone()
            } else {
                b.clone()
            }
        }
        B::BAnd | B::BOr | B::BXor | B::Shl | B::Shr => {
            let bit = match op {
                B::BAnd => BitOp::And,
                B::BOr => BitOp::Or,
                B::BXor => BitOp::Xor,
                B::Shl => BitOp::Shl,
                _ => BitOp::Shr,
            };
            Const::Number(fold_bit(bit, &[num(a)?, num(b)?])?)
        }
    })
}

/// The value of a constant expression (spec §1.3.2), if it is one; `lookup`
/// gives the values of folded constants by name.
pub fn eval(e: &wowlua_syntax::ast::Expr, lookup: &dyn Fn(&str) -> Option<Const>) -> Option<Const> {
    eval_with(e, &|leaf| match &leaf.kind {
        wowlua_syntax::ast::ExprKind::Name(n) => lookup(&n.text),
        _ => None,
    })
}

/// Like [`eval`], with `leaf` resolving names and access paths (fields,
/// indexes) to constants.
/// What [`eval_tracked`] found for parsed expressions, by file and node id.
pub type EvalMemo = rustc_hash::FxHashMap<(wowlua_span::FileId, u32), Option<(Const, bool)>>;

/// [`eval_with`], with whether a leaf gave a value (`true`) or only
/// literals did. A parsed node's answer is kept in `memo` (synthetic nodes
/// have id 0 and are not), so asking again about a subtree of what was
/// asked about takes constant time: `a and b and c` asks about `a and b`,
/// then about `a`.
///
/// Lowering asks within its own recursion, about every operand of a chain,
/// and only the first question goes deep: each level takes stack as it goes
/// (as lowering does), where a `deep::run` would make a stack the size of
/// the whole chain for every question.
pub fn eval_tracked(
    e: &wowlua_syntax::ast::Expr,
    leaf: &dyn Fn(&wowlua_syntax::ast::Expr) -> Option<Const>,
    memo: &mut EvalMemo,
) -> Option<(Const, bool)> {
    eval_tracked_rec(e, leaf, memo)
}

fn eval_tracked_rec(
    e: &wowlua_syntax::ast::Expr,
    leaf: &dyn Fn(&wowlua_syntax::ast::Expr) -> Option<Const>,
    memo: &mut EvalMemo,
) -> Option<(Const, bool)> {
    let key = (e.id.0 != 0).then_some((e.span.file, e.id.0));
    if let Some(key) = key
        && let Some(found) = memo.get(&key)
    {
        return found.clone();
    }
    let found = stacker::maybe_grow(64 * 1024, 1024 * 1024, || eval_tracked_inner(e, leaf, memo));
    if let Some(key) = key {
        memo.insert(key, found.clone());
    }
    found
}

/// As `eval_with_inner`; a value is found by a leaf when any of its parts
/// is (every part was evaluated: a part without a value has none).
fn eval_tracked_inner(
    e: &wowlua_syntax::ast::Expr,
    leaf: &dyn Fn(&wowlua_syntax::ast::Expr) -> Option<Const>,
    memo: &mut EvalMemo,
) -> Option<(Const, bool)> {
    use wowlua_syntax::ast::{ExprKind, UnOp};
    let literal = |c: Const| Some((c, false));
    match &e.kind {
        ExprKind::Nil => literal(Const::Nil),
        ExprKind::True => literal(Const::Bool(true)),
        ExprKind::False => literal(Const::Bool(false)),
        ExprKind::Number(n) => n.value.map(|v| (Const::Number(v), false)),
        ExprKind::String(s) => literal(Const::String(s.value.to_vec())),
        ExprKind::Paren(inner) => eval_tracked_rec(inner, leaf, memo),
        ExprKind::Cast { expr, .. } => eval_tracked_rec(expr, leaf, memo),
        ExprKind::Unary { op, operand } => {
            let (v, found) = eval_tracked_rec(operand, leaf, memo)?;
            let v = match (op, v) {
                (UnOp::Neg, Const::Number(n)) => Const::Number(-n),
                (UnOp::Not, v) => Const::Bool(!v.truthy()),
                (UnOp::BNot, Const::Number(n)) => Const::Number(fold_bnot(n)?),
                _ => return None,
            };
            Some((v, found))
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let (a, fa) = eval_tracked_rec(lhs, leaf, memo)?;
            let (b, fb) = eval_tracked_rec(rhs, leaf, memo)?;
            fold_binary(*op, &a, &b).map(|v| (v, fa || fb))
        }
        _ => leaf(e).map(|v| (v, true)),
    }
}

pub fn eval_with(
    e: &wowlua_syntax::ast::Expr,
    leaf: &dyn Fn(&wowlua_syntax::ast::Expr) -> Option<Const>,
) -> Option<Const> {
    wowlua_syntax::deep::run(|| eval_with_inner(e, leaf))
}

fn eval_with_inner(
    e: &wowlua_syntax::ast::Expr,
    leaf: &dyn Fn(&wowlua_syntax::ast::Expr) -> Option<Const>,
) -> Option<Const> {
    use wowlua_syntax::ast::{ExprKind, UnOp};
    match &e.kind {
        ExprKind::Nil => Some(Const::Nil),
        ExprKind::True => Some(Const::Bool(true)),
        ExprKind::False => Some(Const::Bool(false)),
        ExprKind::Number(n) => n.value.map(Const::Number),
        ExprKind::String(s) => Some(Const::String(s.value.to_vec())),
        ExprKind::Paren(inner) => eval_with_inner(inner, leaf),
        ExprKind::Cast { expr, .. } => eval_with_inner(expr, leaf),
        ExprKind::Unary { op, operand } => {
            let v = eval_with_inner(operand, leaf)?;
            match (op, v) {
                (UnOp::Neg, Const::Number(n)) => Some(Const::Number(-n)),
                (UnOp::Not, v) => Some(Const::Bool(!v.truthy())),
                (UnOp::BNot, Const::Number(n)) => fold_bnot(n).map(Const::Number),
                _ => None,
            }
        }
        ExprKind::Binary { op, lhs, rhs } => {
            let a = eval_with_inner(lhs, leaf)?;
            let b = eval_with_inner(rhs, leaf)?;
            fold_binary(*op, &a, &b)
        }
        _ => leaf(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// After a deep chain was parsed, a question about a small expression
    /// stays on the stack it was asked on: lowering asks about every
    /// operand of a chain, and a stack the size of the chain for each
    /// question took time in the square of its length.
    #[test]
    fn eval_tracked_does_not_grow_a_stack_for_the_whole_chain() {
        let mut db = wowlua_span::SourceDb::new();
        let deep = format!("local b = {}\n", vec!["true"; 5_000].join(" and "));
        let file = db.add("deep.lua", deep.clone());
        let _deep = wowlua_syntax::parse(deep.as_bytes(), file);
        let chain = wowlua_syntax::deep::links();
        assert!(chain >= wowlua_syntax::deep::DEEP, "{chain}");

        let file = db.add("small.lua", "local x = y\n");
        let parse = wowlua_syntax::parse(b"local x = y\n", file);
        let mut name = None;
        wowlua_syntax::visit::walk_block(&parse.chunk.block, &mut |e| {
            if matches!(e.kind, wowlua_syntax::ast::ExprKind::Name(_)) {
                name = name.or(Some(e));
            }
        });
        let before = stacker::remaining_stack().unwrap();
        let at_leaf = std::cell::Cell::new(0);
        let leaf = |_: &wowlua_syntax::ast::Expr| {
            at_leaf.set(stacker::remaining_stack().unwrap());
            None
        };
        eval_tracked(name.unwrap(), &leaf, &mut EvalMemo::default());
        assert!(at_leaf.get() <= before, "{} > {before}", at_leaf.get());
    }

    #[test]
    fn formats_like_lua() {
        assert_eq!(format_g14(1.0), "1");
        assert_eq!(format_g14(0.1), "0.1");
        assert_eq!(format_g14(1e15), "1e+15");
        assert_eq!(format_g14(123456789012345.0), "1.2345678901234e+14");
        assert_eq!(format_g14(0.0001), "0.0001");
        assert_eq!(format_g14(0.00001), "1e-05");
        assert_eq!(format_g14(1.0 / 3.0), "0.33333333333333");
        assert_eq!(format_number(170.0), "170");
        assert_eq!(format_number(0.1), "0.1");
        assert_eq!(format_number(1e300), "1e300");
    }

    #[test]
    fn folds_bitwise_like_luabitop() {
        assert_eq!(
            fold_bit(BitOp::And, &[0xFF as f64, 0x0F as f64]),
            Some(15.0)
        );
        assert_eq!(fold_bit(BitOp::Or, &[1.0, 2.0, 4.0]), Some(7.0));
        assert_eq!(fold_bnot(0.0), Some(-1.0));
        assert_eq!(fold_bit(BitOp::Shr, &[-1.0, 28.0]), Some(15.0));
        assert_eq!(fold_bit(BitOp::Shl, &[1.0, 33.0]), Some(2.0));
        assert_eq!(fold_bit(BitOp::Or, &[4294967295.0, 0.0]), Some(-1.0));
    }

    #[test]
    fn same_compares_numbers_bit_for_bit() {
        assert_eq!(Const::Number(0.0), Const::Number(-0.0));
        assert!(!Const::Number(0.0).same(&Const::Number(-0.0)));
        assert!(Const::Number(f64::NAN).same(&Const::Number(f64::NAN)));
        assert!(Const::Number(1.5).same(&Const::Number(1.5)));
        assert!(Const::String(b"a".to_vec()).same(&Const::String(b"a".to_vec())));
        assert!(!Const::Nil.same(&Const::Bool(false)));
    }

    #[test]
    fn quotes_bytes() {
        assert_eq!(
            quote(b"a\"b\\\n\x01\x012"),
            b"\"a\\\"b\\\\\\n\\1\\0012\"".to_vec()
        );
        assert_eq!(quote(b"\xffO \xc3\xa9"), b"\"\\255O \xc3\xa9\"".to_vec());
    }
}
