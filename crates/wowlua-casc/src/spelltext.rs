//! A spell's description as the client shows it: the placeholders of the
//! client's text (`$s1`, `$d`, `$o2`, …) filled in from the spell's
//! effects, its duration and the caster (spec debugger §18.10, decision
//! T391). A placeholder whose value this cannot give exactly (formulas,
//! conditions, variables, the caster's gender) makes the description
//! unknown (`None`) rather than wrong.

/// What a description's values depend on in the caster.
#[derive(Clone, Copy, Debug, Default)]
pub struct Caster {
    pub level: u32,
    /// Spell power of the spell's school (`GetSpellBonusDamage`).
    pub spell_power: f32,
    /// Bonus healing (`GetSpellBonusHealing`).
    pub healing_power: f32,
    pub attack_power: f32,
    /// What the caster's damage is multiplied by (1: none; 0 is taken as
    /// 1). Damage is shown rounded down after it.
    pub damage_done: f64,
}

/// A spell effect's numbers (`SpellEffect`).
#[derive(Clone, Debug, Default)]
pub struct Effect {
    /// 0-based; placeholders count from 1.
    pub index: u32,
    pub base: f32,
    /// The spread around the value (0.25: 12.5 % below to 12.5 % above).
    pub variance: f32,
    pub per_level: f32,
    /// Spell power coefficient.
    pub sp: f32,
    /// Attack power coefficient.
    pub ap: f32,
    pub period_ms: u32,
    pub chain_targets: u32,
    /// The effect's radius in yards (`SpellRadius`).
    pub radius: Option<f32>,
    /// A heal (its spell power is the caster's healing).
    pub healing: bool,
    /// Damage, at once or over time.
    pub damage: bool,
}

/// A spell's numbers a description reads.
#[derive(Clone, Debug, Default)]
pub struct Data {
    pub duration_ms: Option<i32>,
    /// `SpellLevels`: the level the values are given for, the level they
    /// stop growing at (0: none).
    pub spell_level: u32,
    pub max_level: u32,
    pub effects: Vec<Effect>,
    pub proc_chance: u32,
    pub stacks: u32,
    pub charges: u32,
    pub max_range: f32,
}

/// The spells a description may name (`$12345s1`, `$@spelldesc12345`).
pub trait Spells {
    fn data(&self, id: u32) -> Option<Data>;
    fn name(&self, id: u32) -> Option<String>;
    fn description(&self, id: u32) -> Option<String>;
}

/// `text` of spell `id` with its placeholders filled in; `None` when one
/// cannot be.
pub fn describe(text: &str, id: u32, spells: &dyn Spells, caster: &Caster) -> Option<String> {
    describe_depth(text, id, spells, caster, 0)
}

fn describe_depth(
    text: &str,
    id: u32,
    spells: &dyn Spells,
    caster: &Caster,
    depth: u32,
) -> Option<String> {
    if depth > 3 {
        return None;
    }
    let mut out = String::with_capacity(text.len());
    let mut last_number: Option<f64> = None;
    let b = text.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] != b'$' {
            let ch = text[i..].chars().next()?;
            out.push(ch);
            i += ch.len_utf8();
            continue;
        }
        let mut p = Parser {
            text,
            at: i + 1,
            signed: false,
        };
        match p.placeholder(id, spells, caster, depth, last_number)? {
            Filled::Number(n) => {
                let shown = number(n);
                last_number = shown.replace(',', "").parse().ok();
                out.push_str(&shown);
            }
            Filled::Range(lo, hi) => {
                let (lo, hi) = (lo.round(), hi.round());
                last_number = Some(hi);
                if lo == hi {
                    out.push_str(&number(lo));
                } else {
                    out.push_str(&format!("{} to {}", number(lo), number(hi)));
                }
            }
            Filled::Text(t) => out.push_str(&t),
        }
        i = p.at;
    }
    Some(out)
}

enum Filled {
    /// A number, unrounded.
    Number(f64),
    /// A spread: "low to high".
    Range(f64, f64),
    Text(String),
}

struct Parser<'a> {
    text: &'a str,
    at: usize,
    /// In a formula: values keep their sign (`${$m2/-1000}` of -60000 is
    /// 60); outside one they are shown without it.
    signed: bool,
}

impl Parser<'_> {
    fn peek(&self) -> Option<u8> {
        self.text.as_bytes().get(self.at).copied()
    }

    fn digits(&mut self) -> Option<u32> {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_digit()) {
            self.at += 1;
        }
        (self.at > start).then(|| self.text[start..self.at].parse().ok())?
    }

    fn word(&mut self) -> &str {
        let start = self.at;
        while self.peek().is_some_and(|c| c.is_ascii_alphabetic()) {
            self.at += 1;
        }
        &self.text[start..self.at]
    }

    /// Up to `end`, skipped.
    fn until(&mut self, end: u8) -> Option<&str> {
        let rest = &self.text[self.at..];
        let n = rest.bytes().position(|c| c == end)?;
        self.at += n + 1;
        Some(&rest[..n])
    }

    fn placeholder(
        &mut self,
        id: u32,
        spells: &dyn Spells,
        caster: &Caster,
        depth: u32,
        last: Option<f64>,
    ) -> Option<Filled> {
        match self.peek()? {
            // `${$m1*8}`, `${$s1/1000}.1`: arithmetic of numbers and
            // placeholders (`+ - * /`, parentheses), shown whole, else to
            // the decimals after it; anything else in it (a condition, a
            // function, a variable) is not filled
            b'{' => {
                self.at += 1;
                let rest = &self.text[self.at..];
                let end = rest.find('}')?;
                let expr = &rest[..end];
                self.at += end + 1;
                let v = Formula {
                    text: expr,
                    at: 0,
                    id,
                    spells,
                    caster,
                    depth,
                }
                .eval()?;
                // `.1`: one decimal (a digit right after the brace's dot)
                let b = self.text.as_bytes();
                if b.get(self.at) == Some(&b'.')
                    && b.get(self.at + 1).is_some_and(u8::is_ascii_digit)
                    && !b.get(self.at + 2).is_some_and(u8::is_ascii_digit)
                {
                    let places = usize::from(b[self.at + 1] - b'0');
                    self.at += 2;
                    return Some(Filled::Text(format!("{v:.places$}")));
                }
                Some(Filled::Number(v.round()))
            }
            // `$/1000;S1`, `$*2;s1`: a value divided or multiplied
            op @ (b'/' | b'*') => {
                self.at += 1;
                let by: f64 = self.until(b';')?.trim().parse().ok()?;
                let scale = |n: f64| if op == b'/' { n / by } else { n * by };
                match self.placeholder(id, spells, caster, depth, last)? {
                    Filled::Number(n) => Some(Filled::Number(scale(n))),
                    Filled::Range(lo, hi) => Some(Filled::Range(scale(lo), scale(hi))),
                    Filled::Text(_) => None,
                }
            }
            // `$@spelldesc123`, `$@spellname123`
            b'@' => {
                self.at += 1;
                let what = self.word().to_ascii_lowercase();
                let other = self.digits().unwrap_or(id);
                match what.as_str() {
                    "spellname" => Some(Filled::Text(spells.name(other)?)),
                    "spelldesc" => {
                        let text = spells.description(other)?;
                        Some(Filled::Text(describe_depth(
                            &text,
                            other,
                            spells,
                            caster,
                            depth + 1,
                        )?))
                    }
                    _ => None,
                }
            }
            c if c.is_ascii_digit() || c.is_ascii_alphabetic() => {
                let other = self.digits();
                let word = self.word().to_string();
                // `$lsingular:plural;`: by the number before it
                if let Some(rest) = word.strip_prefix('l').filter(|_| other.is_none())
                    && !rest.is_empty()
                {
                    self.at -= rest.len();
                    let forms = self.until(b';')?;
                    let (one, many) = forms.split_once(':')?;
                    let n = last?;
                    return Some(Filled::Text(if n == 1.0 { one } else { many }.to_string()));
                }
                let index = self.digits();
                let spell = other.unwrap_or(id);
                value(&word, index, spell, spells, caster, self.signed)
            }
            _ => None,
        }
    }
}

/// A `${…}` formula: sums of products of numbers, placeholders and
/// parenthesized formulas.
struct Formula<'a> {
    text: &'a str,
    at: usize,
    id: u32,
    spells: &'a dyn Spells,
    caster: &'a Caster,
    depth: u32,
}

impl Formula<'_> {
    fn eval(mut self) -> Option<f64> {
        let v = self.sum()?;
        self.skip();
        (self.at == self.text.len()).then_some(v)
    }

    fn skip(&mut self) {
        while self.text.as_bytes().get(self.at) == Some(&b' ') {
            self.at += 1;
        }
    }

    fn sum(&mut self) -> Option<f64> {
        let mut v = self.product()?;
        loop {
            self.skip();
            match self.text.as_bytes().get(self.at) {
                Some(b'+') => {
                    self.at += 1;
                    v += self.product()?;
                }
                Some(b'-') => {
                    self.at += 1;
                    v -= self.product()?;
                }
                _ => return Some(v),
            }
        }
    }

    fn product(&mut self) -> Option<f64> {
        let mut v = self.atom()?;
        loop {
            self.skip();
            match self.text.as_bytes().get(self.at) {
                Some(b'*') => {
                    self.at += 1;
                    v *= self.atom()?;
                }
                Some(b'/') => {
                    self.at += 1;
                    let d = self.atom()?;
                    (d != 0.0).then_some(())?;
                    v /= d;
                }
                _ => return Some(v),
            }
        }
    }

    fn atom(&mut self) -> Option<f64> {
        self.skip();
        let b = self.text.as_bytes();
        match *b.get(self.at)? {
            b'(' => {
                self.at += 1;
                let v = self.sum()?;
                self.skip();
                (self.text.as_bytes().get(self.at) == Some(&b')')).then_some(())?;
                self.at += 1;
                Some(v)
            }
            b'-' => {
                self.at += 1;
                Some(-self.atom()?)
            }
            b'$' => {
                let mut p = Parser {
                    text: self.text,
                    at: self.at + 1,
                    signed: true,
                };
                let v = match p.placeholder(self.id, self.spells, self.caster, self.depth, None)? {
                    Filled::Number(n) => n,
                    _ => return None,
                };
                self.at = p.at;
                Some(v)
            }
            c if c.is_ascii_digit() || c == b'.' => {
                let start = self.at;
                while b
                    .get(self.at)
                    .is_some_and(|c| c.is_ascii_digit() || *c == b'.')
                {
                    self.at += 1;
                }
                self.text[start..self.at].parse().ok()
            }
            _ => None,
        }
    }
}

/// A placeholder's value: `$s1` and the like.
fn value(
    word: &str,
    index: Option<u32>,
    spell: u32,
    spells: &dyn Spells,
    caster: &Caster,
    signed: bool,
) -> Option<Filled> {
    let shown = |v: f64| if signed { v } else { v.abs() };
    let data = spells.data(spell)?;
    let effect = |i: Option<u32>| {
        let i = i.unwrap_or(1).checked_sub(1)?;
        data.effects.iter().find(|e| e.index == i)
    };
    let num = |v: f64| Some(Filled::Number(v));
    match word {
        // the effect's points; with a spread, "low to high"
        "s" | "S" => {
            let e = effect(index)?;
            let v = shown(points(e, &data, caster)?);
            if e.variance > 0.0 {
                let half = f64::from(e.variance) / 2.0;
                return Some(Filled::Range(v * (1.0 - half), v * (1.0 + half)));
            }
            num(down(v))
        }
        "m" => {
            let e = effect(index)?;
            let v = shown(points(e, &data, caster)?);
            num((v * (1.0 - f64::from(e.variance) / 2.0)).round())
        }
        "M" => {
            let e = effect(index)?;
            let v = shown(points(e, &data, caster)?);
            num((v * (1.0 + f64::from(e.variance) / 2.0)).round())
        }
        // over the whole duration: each tick's points times the ticks
        "o" => {
            let e = effect(index)?;
            let duration = data.duration_ms.filter(|d| *d > 0)?;
            if e.period_ms == 0 || e.variance > 0.0 {
                return None;
            }
            let ticks = f64::from(duration) / f64::from(e.period_ms);
            let total = shown(points(e, &data, caster)?) * ticks.floor();
            num(if e.damage { down(total) } else { total.round() })
        }
        // the period in seconds
        "t" => {
            let e = effect(index)?;
            (e.period_ms > 0).then_some(())?;
            num(f64::from(e.period_ms) / 1000.0)
        }
        "d" | "D" => {
            let d = data.duration_ms.filter(|d| *d > 0)?;
            Some(Filled::Text(duration(d)?))
        }
        "a" | "A" => num(f64::from(effect(index)?.radius?)),
        "h" => num(f64::from(data.proc_chance)),
        "u" => num(f64::from(data.stacks)),
        "n" => num(f64::from(data.charges)),
        "x" => num(f64::from(effect(index)?.chain_targets)),
        "r" => num(f64::from(data.max_range)),
        _ => None,
    }
}

/// An effect's points for the caster: its base, grown by its points per
/// level from the spell's level up to its last, and its coefficients of
/// spell and attack power.
fn points(e: &Effect, data: &Data, caster: &Caster) -> Option<f64> {
    let mut level = caster.level;
    if data.max_level > 0 {
        level = level.min(data.max_level);
    }
    let grown = level.saturating_sub(data.spell_level);
    let factor = if e.damage && caster.damage_done > 0.0 {
        caster.damage_done
    } else {
        1.0
    };
    Some(
        factor
            * (f64::from(e.base)
                + f64::from(e.per_level) * f64::from(grown)
                + f64::from(e.sp)
                    * f64::from(if e.healing {
                        caster.healing_power
                    } else {
                        caster.spell_power
                    })
                + f64::from(e.ap) * f64::from(caster.attack_power)),
    )
}

/// Rounded down, past the float error of a product (10.2 × 5 is 51; the
/// tables' values are single floats).
fn down(v: f64) -> f64 {
    (v + 1e-4).floor()
}

/// A number as descriptions show it: whole, else to one decimal; from
/// 1,000 up with a thousands separator.
fn number(v: f64) -> String {
    let r = (v * 10.0).round() / 10.0;
    let whole = r.trunc().abs() as i64;
    let mut digits = whole.to_string();
    let mut i = digits.len();
    while i > 3 {
        i -= 3;
        digits.insert(i, ',');
    }
    let sign = if r < 0.0 { "-" } else { "" };
    if r.fract() == 0.0 {
        format!("{sign}{digits}")
    } else {
        let tenth = ((r.abs().fract() * 10.0).round() as i64).min(9);
        format!("{sign}{digits}.{tenth}")
    }
}

/// A duration as descriptions show it: "12 sec", "1.5 sec", "10 min",
/// "1 |4hour:hrs;", "2 |4day:days;". `None` for one that is not whole in
/// its unit (unseen in the client's texts).
fn duration(ms: i32) -> Option<String> {
    let ms = i64::from(ms);
    const MIN: i64 = 60_000;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    if ms >= DAY {
        return (ms % DAY == 0).then(|| format!("{} |4day:days;", ms / DAY));
    }
    if ms >= HOUR {
        return (ms % HOUR == 0).then(|| format!("{} |4hour:hrs;", ms / HOUR));
    }
    if ms >= MIN {
        return (ms % MIN == 0).then(|| format!("{} min", ms / MIN));
    }
    Some(format!("{} sec", number(ms as f64 / 1000.0)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct Book(HashMap<u32, (String, String, Data)>);

    impl Spells for Book {
        fn data(&self, id: u32) -> Option<Data> {
            self.0.get(&id).map(|s| s.2.clone())
        }
        fn name(&self, id: u32) -> Option<String> {
            self.0.get(&id).map(|s| s.0.clone())
        }
        fn description(&self, id: u32) -> Option<String> {
            self.0.get(&id).map(|s| s.1.clone())
        }
    }

    fn moonfire() -> Data {
        Data {
            duration_ms: Some(9000),
            spell_level: 4,
            max_level: 0,
            effects: vec![
                Effect {
                    index: 0,
                    base: 4.0,
                    period_ms: 3000,
                    ..Default::default()
                },
                Effect {
                    index: 1,
                    base: 8.0,
                    variance: 0.25,
                    per_level: 0.5,
                    ..Default::default()
                },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn placeholders_are_filled_as_the_client_does() {
        let text = "Burns the enemy for $s2 Arcane damage and then an additional $o1 Arcane damage over $d.";
        let book = Book([(8921, ("Moonfire".into(), text.into(), moonfire()))].into());
        let at = |level| {
            describe(
                text,
                8921,
                &book,
                &Caster {
                    level,
                    ..Default::default()
                },
            )
        };
        // recorded at a low level: "7 to 9", "12", "9 sec"
        assert_eq!(
            at(4).as_deref(),
            Some(
                "Burns the enemy for 7 to 9 Arcane damage and then an additional 12 Arcane damage over 9 sec."
            )
        );
        assert_eq!(duration(3_600_000).as_deref(), Some("1 |4hour:hrs;"));
        assert_eq!(duration(600_000).as_deref(), Some("10 min"));
        assert_eq!(duration(1500).as_deref(), Some("1.5 sec"));
        // what cannot be given exactly gives nothing
        assert_eq!(
            describe(
                "${$m2*2} and ${($s1+1)/2}.1",
                8921,
                &book,
                &Caster::default()
            )
            .as_deref(),
            Some("14 and 2.5")
        );
        assert_eq!(
            describe("${$cond(1,2)}", 8921, &book, &Caster::default()),
            None
        );
        assert_eq!(
            describe("$?s123[a][b]", 8921, &book, &Caster::default()),
            None
        );
        assert_eq!(
            describe("$ghis:her;", 8921, &book, &Caster::default()),
            None
        );
        // plurals, other spells, scaled values
        assert_eq!(describe("$/1000;t2", 8921, &book, &Caster::default()), None);
        assert_eq!(
            describe(
                "$s1 $lpoint:points; of $@spellname8921 every $t1 sec",
                8921,
                &book,
                &Caster::default()
            )
            .as_deref(),
            Some("4 points of Moonfire every 3 sec")
        );
    }
}
