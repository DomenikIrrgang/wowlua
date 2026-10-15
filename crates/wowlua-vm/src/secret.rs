//! Secret values (spec emulator §17.5, decision T195), as the
//! client has them (probed with `/stdtest probe`): a secret keeps its type,
//! which code may always see, but tainted code may not read the value. So
//! `type`, truth tests of numbers, strings and tables, and equality with a
//! value of another type work; arithmetic, comparisons, indexing, calls
//! and conversions in library functions fail with errors naming the
//! execution's taint. Secure code may use secrets, and what it computes from
//! one is secret again (decision T195, from Blizzard's documentation; addons
//! cannot observe it).

use crate::error::LuaError;
use crate::value::Value;
use crate::vm::Vm;

impl Vm {
    /// `v` made secret (`secretwrap`); a secret stays as it is.
    pub fn secret_wrap(&mut self, v: Value) -> Value {
        match v {
            Value::Secret(_) => v,
            _ => Value::Secret(self.heap.alloc_secret(v)),
        }
    }

    /// The value a secret hides, or `v` itself.
    pub fn hidden(&self, v: Value) -> Value {
        match v {
            Value::Secret(s) => self.heap.secret(s),
            _ => v,
        }
    }

    /// The name `type()` gives: a secret's hidden value's.
    pub fn type_of(&self, v: Value) -> &'static str {
        self.hidden(v).type_name()
    }

    /// Whether the running code may use secrets: its execution is secure,
    /// and its function has not given the access up (`dropsecretaccess`).
    pub fn can_access_secrets(&self) -> bool {
        self.cur.taint.stack.is_secure()
            && !self.cur.frames.last().is_some_and(|f| f.secrets_dropped)
    }

    /// "execution tainted by 'X'", as the client's secret errors end.
    fn tainted_by(&self) -> String {
        let name = self.taint_name(self.cur.taint.stack).unwrap_or_default();
        format!("execution tainted by '{name}'")
    }

    /// An operation on a secret refused: with the operand's name, "attempt
    /// to OP KIND 'NAME' (a secret T value, while execution tainted by
    /// 'X')"; without, "attempt to OP a secret T value (execution tainted
    /// by 'X')".
    pub(crate) fn secret_error(&mut self, v: Value, op: &str, reg: Option<u8>) -> LuaError {
        let t = self.type_of(v);
        let by = self.tainted_by();
        let msg = match reg.and_then(|r| self.reg_name(r)) {
            Some((kind, name)) => {
                format!("attempt to {op} {kind} '{name}' (a secret {t} value, while {by})")
            }
            None => format!("attempt to {op} a secret {t} value ({by})"),
        };
        self.rt_error(&msg)
    }

    /// A library function's conversion of a secret refused: `kind` is
    /// "string" or "numeric".
    pub(crate) fn secret_conversion_error(&mut self, v: Value, kind: &str) -> LuaError {
        self.secret_error(v, &format!("perform {kind} conversion on"), None)
    }

    /// A secret table key refused (`assign`: a write).
    pub(crate) fn secret_key_error(&mut self, assign: bool) -> LuaError {
        let op = if assign {
            "perform indexed assignment on"
        } else {
            "index"
        };
        self.rt_error(&format!(
            "attempted to {op} a table that cannot be indexed with secret keys"
        ))
    }

    /// `x == y` with a secret: values of different types differ, two nils
    /// are equal, and otherwise the values are needed (refused while
    /// tainted, `blame` names the secret operand's register).
    pub(crate) fn secret_equals(
        &mut self,
        x: Value,
        y: Value,
        regs: (Option<u8>, Option<u8>),
    ) -> Result<bool, LuaError> {
        let (tx, ty) = (self.type_of(x), self.type_of(y));
        if tx != ty {
            return Ok(false);
        }
        if tx == "nil" {
            return Ok(true);
        }
        if self.can_access_secrets() {
            let (hx, hy) = (self.hidden(x), self.hidden(y));
            return self.equals(hx, hy);
        }
        Err(self.blame_secret(x, y, "compare", regs))
    }

    /// The error for the first secret of two operands.
    pub(crate) fn blame_secret(
        &mut self,
        x: Value,
        y: Value,
        op: &str,
        regs: (Option<u8>, Option<u8>),
    ) -> LuaError {
        if matches!(x, Value::Secret(_)) {
            self.secret_error(x, op, regs.0)
        } else {
            self.secret_error(y, op, regs.1)
        }
    }

    /// A truth test of a secret: a boolean's is refused while tainted.
    pub(crate) fn secret_truthy(
        &mut self,
        v: Value,
        op: &str,
        reg: Option<u8>,
    ) -> Result<bool, LuaError> {
        let h = self.hidden(v);
        if matches!(h, Value::Bool(_)) && !self.can_access_secrets() {
            return Err(self.secret_error(v, op, reg));
        }
        Ok(h.truthy())
    }

    /// A secret argument a library function converts: its value where
    /// secrets may be used (the results become secret), nil for a secret
    /// nil, and otherwise the conversion's error.
    pub(crate) fn secret_arg(&mut self, v: Value, kind: &str) -> Result<Value, LuaError> {
        let h = self.hidden(v);
        if h.is_nil() {
            return Ok(h);
        }
        if self.can_access_secrets() || self.secret_reveal {
            self.secret_used = true;
            return Ok(h);
        }
        Err(self.secret_conversion_error(v, kind))
    }

    /// Whether an operation on `x` and `y` involves a secret the running
    /// code may use (then it runs on the hidden values, and its result is
    /// secret).
    pub(crate) fn secret_usable(&self, x: Value, y: Value) -> bool {
        (matches!(x, Value::Secret(_)) || matches!(y, Value::Secret(_)))
            && self.can_access_secrets()
    }

    /// `x .. y` with a secret, the operands in registers `rx` and `ry`.
    pub(crate) fn secret_concat(
        &mut self,
        x: Value,
        y: Value,
        rx: usize,
        ry: usize,
    ) -> Result<Value, LuaError> {
        let regs = (u8::try_from(rx).ok(), u8::try_from(ry).ok());
        self.secret_concat_values(x, y, regs)
    }

    /// `x .. y` with a secret: secret strings and numbers join into a secret
    /// string even while tainted; anything else blames the secret.
    pub(crate) fn secret_concat_values(
        &mut self,
        x: Value,
        y: Value,
        regs: (Option<u8>, Option<u8>),
    ) -> Result<Value, LuaError> {
        let (hx, hy) = (self.hidden(x), self.hidden(y));
        let strlike = |v: Value| matches!(v, Value::Str(_) | Value::Num(_));
        if strlike(hx) && strlike(hy) {
            let mut buf = Vec::new();
            self.append_strlike(&mut buf, hx);
            self.append_strlike(&mut buf, hy);
            let s = self.bytes(&buf);
            return Ok(self.secret_wrap(s));
        }
        if self.can_access_secrets() {
            let r = self.concat_values(hx, hy)?;
            return Ok(self.secret_wrap(r));
        }
        Err(self.blame_secret(x, y, "concatenate", regs))
    }

    /// `t[key]` where the table or the key is secret.
    pub(crate) fn secret_index(
        &mut self,
        t: Value,
        key: Value,
        reg: Option<u8>,
    ) -> Result<crate::taint::Slot, LuaError> {
        if !self.can_access_secrets() {
            if let Value::Secret(_) = t {
                return Err(self.secret_error(t, "index", reg));
            }
            if matches!(t, Value::Table(_)) {
                return Err(self.secret_key_error(false));
            }
        }
        let (ht, hk) = (self.hidden(t), self.hidden(key));
        let r = self.index(ht, hk, reg)?;
        let r = self.secret_wrap(r);
        Ok(self.cur.taint.fresh(r))
    }

    /// `t[key] = v` where the table or the key is secret.
    pub(crate) fn secret_set_index(
        &mut self,
        t: Value,
        key: Value,
        v: crate::taint::Slot,
        reg: Option<u8>,
    ) -> Result<(), LuaError> {
        if !self.can_access_secrets() {
            if let Value::Secret(_) = t {
                return Err(self.secret_error(t, "perform indexed assignment on", reg));
            }
            if matches!(t, Value::Table(_)) {
                return Err(self.secret_key_error(true));
            }
        }
        let (ht, hk) = (self.hidden(t), self.hidden(key));
        self.set_index_slot(ht, hk, v, reg)
    }

    /// A key a raw access uses: a secret one is refused while tainted, and
    /// otherwise its value is the key.
    pub(crate) fn secret_key(&mut self, k: Value, assign: bool) -> Result<Value, LuaError> {
        match k {
            Value::Secret(_) if !self.can_access_secrets() => Err(self.secret_key_error(assign)),
            Value::Secret(_) => Ok(self.hidden(k)),
            _ => Ok(k),
        }
    }

    /// A FrameScript function's refusal of a secret argument: its usage,
    /// with neither the caller's position nor the function's name.
    pub(crate) fn secret_usage_error(&mut self, n: usize, usage: &str) -> LuaError {
        let msg = format!(
            "bad argument #{n} to '?' ({usage}. Secret values are only allowed during untainted execution for this argument.)"
        );
        let v = self.str(&msg);
        self.raise(v)
    }
}
