//! Number formatting and parsing as the client's C library does it: `%.14g`
//! for `tostring`, `strtod` (with hexadecimal, `inf` and `nan`) for
//! `tonumber` and arithmetic on strings.

/// C's `%.<precision>g`.
pub fn fmt_g(n: f64, precision: usize, upper: bool, alt: bool) -> String {
    if n.is_nan() {
        return if upper { "NAN" } else { "nan" }.into();
    }
    if n.is_infinite() {
        let s = if n < 0.0 { "-inf" } else { "inf" };
        return if upper { s.to_uppercase() } else { s.into() };
    }
    let p = precision.max(1);
    if n == 0.0 {
        let mut s = if n.is_sign_negative() {
            "-0".to_string()
        } else {
            "0".to_string()
        };
        if alt {
            s.push('.');
            s.extend(std::iter::repeat_n('0', p - 1));
        }
        return s;
    }
    // The exponent after rounding to `p` significant digits.
    let sci = format!("{:.*e}", p - 1, n);
    let (_, exp) = sci.split_once('e').expect("exponent");
    let x: i32 = exp.parse().expect("exponent digits");
    let mut out = if x < -4 || x >= p as i32 {
        let (mant, _) = sci.split_once('e').expect("exponent");
        let mut m = mant.to_string();
        if !alt {
            trim_zeros(&mut m);
        }
        let sign = if x < 0 { '-' } else { '+' };
        format!("{m}e{sign}{:02}", x.abs())
    } else {
        let decimals = (p as i32 - 1 - x).max(0) as usize;
        let mut m = format!("{:.*}", decimals, n);
        if !alt {
            trim_zeros(&mut m);
        }
        m
    };
    if upper {
        out = out.to_uppercase();
    }
    out
}

fn trim_zeros(s: &mut String) {
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
}

/// `tostring` of a number (`LUA_NUMBER_FMT` `%.14g`).
pub fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 && n.abs() < 1e14 {
        if n == 0.0 && n.is_sign_negative() {
            return "-0".into();
        }
        return format!("{}", n as i64);
    }
    fmt_g(n, 14, false, false)
}

fn is_space(b: u8) -> bool {
    matches!(b, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/// Lua 5.1's `luaO_str2d`: the whole string (but surrounding spaces) must
/// be a number.
pub fn str2num(s: &[u8]) -> Option<f64> {
    let (v, used) = strtod(s)?;
    if s[used..].iter().all(|&b| is_space(b)) {
        Some(v)
    } else {
        None
    }
}

/// C99 `strtod`: the value and the bytes consumed (none: no number).
pub fn strtod(s: &[u8]) -> Option<(f64, usize)> {
    let mut i = 0;
    while i < s.len() && is_space(s[i]) {
        i += 1;
    }
    let mut neg = false;
    if i < s.len() && (s[i] == b'+' || s[i] == b'-') {
        neg = s[i] == b'-';
        i += 1;
    }
    let rest = &s[i..];
    let lower = |n: usize| -> Vec<u8> { rest.iter().take(n).map(u8::to_ascii_lowercase).collect() };
    let sign = |v: f64| if neg { -v } else { v };
    if lower(8) == b"infinity" {
        return Some((sign(f64::INFINITY), i + 8));
    }
    if lower(3) == b"inf" {
        return Some((sign(f64::INFINITY), i + 3));
    }
    if lower(3) == b"nan" {
        let mut j = i + 3;
        // `nan(chars)`
        if j < s.len() && s[j] == b'(' {
            let mut k = j + 1;
            while k < s.len() && (s[k].is_ascii_alphanumeric() || s[k] == b'_') {
                k += 1;
            }
            if k < s.len() && s[k] == b')' {
                j = k + 1;
            }
        }
        return Some((sign(f64::NAN), j));
    }
    if rest.len() >= 2 && rest[0] == b'0' && (rest[1] == b'x' || rest[1] == b'X') {
        if let Some((v, n)) = hex(&rest[2..]) {
            return Some((sign(v), i + 2 + n));
        }
        // "0x" without digits: just the "0".
        return Some((sign(0.0), i + 1));
    }
    let start = i;
    let mut j = i;
    let mut digits = 0;
    while j < s.len() && s[j].is_ascii_digit() {
        j += 1;
        digits += 1;
    }
    if j < s.len() && s[j] == b'.' {
        j += 1;
        while j < s.len() && s[j].is_ascii_digit() {
            j += 1;
            digits += 1;
        }
    }
    if digits == 0 {
        return None;
    }
    if j < s.len() && (s[j] == b'e' || s[j] == b'E') {
        let mut k = j + 1;
        if k < s.len() && (s[k] == b'+' || s[k] == b'-') {
            k += 1;
        }
        if k < s.len() && s[k].is_ascii_digit() {
            while k < s.len() && s[k].is_ascii_digit() {
                k += 1;
            }
            j = k;
        }
    }
    let text = std::str::from_utf8(&s[start..j]).ok()?;
    let text = if text.ends_with('.') && text.len() > 1 {
        &text[..text.len() - 1]
    } else {
        text
    };
    let text = if let Some(rest) = text.strip_prefix('.') {
        format!("0.{rest}")
    } else {
        text.replace(".e", "e").replace(".E", "E")
    };
    let v: f64 = text.parse().ok()?;
    Some((sign(v), j))
}

/// A hexadecimal floating literal after `0x`: digits, an optional fraction
/// and an optional binary exponent.
fn hex(s: &[u8]) -> Option<(f64, usize)> {
    let mut v = 0.0f64;
    let mut i = 0;
    let mut digits = 0;
    while i < s.len() && s[i].is_ascii_hexdigit() {
        v = v * 16.0 + f64::from((s[i] as char).to_digit(16)?);
        i += 1;
        digits += 1;
    }
    let mut scale = 0i32;
    if i < s.len() && s[i] == b'.' {
        let mut j = i + 1;
        while j < s.len() && s[j].is_ascii_hexdigit() {
            v = v * 16.0 + f64::from((s[j] as char).to_digit(16)?);
            scale -= 4;
            j += 1;
            digits += 1;
        }
        i = j;
    }
    if digits == 0 {
        return None;
    }
    if i < s.len() && (s[i] == b'p' || s[i] == b'P') {
        let mut k = i + 1;
        let mut eneg = false;
        if k < s.len() && (s[k] == b'+' || s[k] == b'-') {
            eneg = s[k] == b'-';
            k += 1;
        }
        if k < s.len() && s[k].is_ascii_digit() {
            let mut e = 0i32;
            while k < s.len() && s[k].is_ascii_digit() {
                e = e.saturating_mul(10).saturating_add(i32::from(s[k] - b'0'));
                k += 1;
            }
            scale += if eneg { -e } else { e };
            i = k;
        }
    }
    Some((v * 2f64.powi(scale), i))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_printf() {
        assert_eq!(fmt_num(1.0), "1");
        assert_eq!(fmt_num(-0.0), "-0");
        assert_eq!(fmt_num(0.1), "0.1");
        assert_eq!(fmt_num(1.0 / 3.0), "0.33333333333333");
        assert_eq!(fmt_num(1e15), "1e+15");
        assert_eq!(fmt_num(123456789012345.0), "1.2345678901234e+14");
        assert_eq!(fmt_num(1e-5), "1e-05");
        assert_eq!(fmt_num(0.0001), "0.0001");
        assert_eq!(fmt_num(2.5e300), "2.5e+300");
        assert_eq!(fmt_num(f64::INFINITY), "inf");
        assert_eq!(fmt_g(100.0, 1, false, false), "1e+02");
    }

    #[test]
    fn parses_like_strtod() {
        assert_eq!(str2num(b" 10 "), Some(10.0));
        assert_eq!(str2num(b"0x10"), Some(16.0));
        assert_eq!(str2num(b"-0x10"), Some(-16.0));
        assert_eq!(str2num(b"1e2"), Some(100.0));
        assert_eq!(str2num(b".5"), Some(0.5));
        assert_eq!(str2num(b"5."), Some(5.0));
        assert_eq!(str2num(b"1e"), None);
        assert_eq!(str2num(b"abc"), None);
        assert_eq!(str2num(b""), None);
        assert!(str2num(b"nan").is_some_and(f64::is_nan));
        assert_eq!(str2num(b"inf"), Some(f64::INFINITY));
    }
}
