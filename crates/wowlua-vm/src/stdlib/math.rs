//! Lua 5.1's math library (`lmathlib.c`). `random` is deterministic (a
//! seeded generator): C's `rand` differs per platform, so no test can rely
//! on its numbers anyway.

use crate::function::NativeCall;
use crate::stdlib::{Ret, register};
use crate::value::Value;
use crate::vm::Vm;

const RADIANS_PER_DEGREE: f64 = std::f64::consts::PI / 180.0;
const RAND_MAX: u32 = 0x7fff_ffff;

pub fn open(vm: &mut Vm) {
    let t = register(
        vm,
        "math",
        &[
            ("abs", abs),
            ("ceil", ceil),
            ("floor", floor),
            ("sqrt", sqrt),
            ("sin", sin),
            ("cos", cos),
            ("tan", tan),
            ("asin", asin),
            ("acos", acos),
            ("atan", atan),
            ("atan2", atan2),
            ("sinh", sinh),
            ("cosh", cosh),
            ("tanh", tanh),
            ("exp", exp),
            ("log", log),
            ("log10", log10),
            ("pow", pow),
            ("fmod", fmod),
            ("modf", modf),
            ("frexp", frexp),
            ("ldexp", ldexp),
            ("deg", deg),
            ("rad", rad),
            ("min", min),
            ("max", max),
            ("random", random),
            ("randomseed", randomseed),
        ],
    );
    let pi = vm.heap.intern(b"pi");
    let huge = vm.heap.intern(b"huge");
    let tm = vm.heap.table_mut(t);
    tm.set_str(pi, Value::Num(std::f64::consts::PI));
    tm.set_str(huge, Value::Num(f64::INFINITY));
}

macro_rules! unary {
    ($($name:ident => $f:expr),* $(,)?) => {
        $(fn $name(vm: &mut Vm, c: NativeCall) -> Ret {
            let x = vm.check_number(&c, 1)?;
            let f: fn(f64) -> f64 = $f;
            Ok(vec![Value::Num(f(x))])
        })*
    };
}

unary! {
    abs => f64::abs,
    ceil => f64::ceil,
    floor => f64::floor,
    sqrt => f64::sqrt,
    sin => f64::sin,
    cos => f64::cos,
    tan => f64::tan,
    asin => f64::asin,
    acos => f64::acos,
    atan => f64::atan,
    sinh => f64::sinh,
    cosh => f64::cosh,
    tanh => f64::tanh,
    exp => f64::exp,
    log => f64::ln,
    log10 => f64::log10,
    deg => |x| x / RADIANS_PER_DEGREE,
    rad => |x| x * RADIANS_PER_DEGREE,
}

fn atan2(vm: &mut Vm, c: NativeCall) -> Ret {
    let y = vm.check_number(&c, 1)?;
    let x = vm.check_number(&c, 2)?;
    Ok(vec![Value::Num(y.atan2(x))])
}

fn pow(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = vm.check_number(&c, 1)?;
    let y = vm.check_number(&c, 2)?;
    Ok(vec![Value::Num(x.powf(y))])
}

fn fmod(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = vm.check_number(&c, 1)?;
    let y = vm.check_number(&c, 2)?;
    Ok(vec![Value::Num(x % y)])
}

fn modf(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = vm.check_number(&c, 1)?;
    let ip = x.trunc();
    let fp = if x.is_infinite() {
        0.0f64.copysign(x)
    } else {
        x - ip
    };
    Ok(vec![Value::Num(ip), Value::Num(fp)])
}

fn frexp(vm: &mut Vm, c: NativeCall) -> Ret {
    let x = vm.check_number(&c, 1)?;
    if x == 0.0 || !x.is_finite() {
        return Ok(vec![Value::Num(x), Value::Num(0.0)]);
    }
    let mut e = x.abs().log2().floor() as i32 + 1;
    let mut m = x / 2f64.powi(e);
    // Correct rounding at the edges of the binade.
    if m.abs() >= 1.0 {
        m /= 2.0;
        e += 1;
    } else if m.abs() < 0.5 {
        m *= 2.0;
        e -= 1;
    }
    Ok(vec![Value::Num(m), Value::Num(f64::from(e))])
}

fn ldexp(vm: &mut Vm, c: NativeCall) -> Ret {
    let m = vm.check_number(&c, 1)?;
    let e = vm.check_int(&c, 2)? as i32;
    Ok(vec![Value::Num(m * 2f64.powi(e))])
}

fn min(vm: &mut Vm, c: NativeCall) -> Ret {
    let mut m = vm.check_number(&c, 1)?;
    for i in 2..=c.len() {
        let d = vm.check_number(&c, i)?;
        if d < m {
            m = d;
        }
    }
    Ok(vec![Value::Num(m)])
}

fn max(vm: &mut Vm, c: NativeCall) -> Ret {
    let mut m = vm.check_number(&c, 1)?;
    for i in 2..=c.len() {
        let d = vm.check_number(&c, i)?;
        if d > m {
            m = d;
        }
    }
    Ok(vec![Value::Num(m)])
}

impl Vm {
    /// C's `rand()`: a 31-bit linear congruential generator.
    pub(crate) fn rand(&mut self) -> u32 {
        self.rand_state = self
            .rand_state
            .wrapping_mul(1_103_515_245)
            .wrapping_add(12345);
        (self.rand_state >> 1) & RAND_MAX
    }
}

pub(crate) fn random(vm: &mut Vm, c: NativeCall) -> Ret {
    let r = f64::from(vm.rand() % RAND_MAX) / f64::from(RAND_MAX);
    match c.len() {
        0 => Ok(vec![Value::Num(r)]),
        1 => {
            let u = vm.check_int(&c, 1)?;
            if u < 1 {
                return Err(vm.arg_error(1, "interval is empty"));
            }
            Ok(vec![Value::Num((r * u as f64).floor() + 1.0)])
        }
        2 => {
            let l = vm.check_int(&c, 1)?;
            let u = vm.check_int(&c, 2)?;
            if l > u {
                return Err(vm.arg_error(2, "interval is empty"));
            }
            Ok(vec![Value::Num(
                (r * (u - l + 1) as f64).floor() + l as f64,
            )])
        }
        _ => Err(vm.error("wrong number of arguments")),
    }
}

fn randomseed(vm: &mut Vm, c: NativeCall) -> Ret {
    let s = vm.check_int(&c, 1)?;
    vm.rand_state = s as u32;
    Ok(vec![])
}
