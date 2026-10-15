//! Where taint came from (spec emulator §17.4, decision T196): every tainted
//! value and execution knows the event that explains its taint, and those
//! events chain back to the addon code behind them. A *read* is secure code
//! becoming tainted by a value; a *write* is a tainted value stored into a
//! table or an upvalue, with what tainted it then. A report walks the chain
//! from a refused action (or a failed assertion) back to the addon's line.
//! Records are kept per site and cause, so they stay few.

use std::collections::HashMap;
use std::rc::Rc;

use crate::proto::{Op, Proto, Rk};
use crate::taint::{Origin, Taint};
use crate::value::Value;
use crate::vm::Vm;

/// Where something happened: the function, as stack traces name it, its
/// source and line, and what was read or written there.
#[derive(Clone, Debug)]
pub struct Site {
    pub func: String,
    pub source: String,
    pub line: u32,
    /// "global 'X'", "field 'f'", "upvalue 'u'", "method 'm'".
    pub what: String,
}

#[derive(Clone, Debug)]
pub enum Record {
    /// Secure code became tainted reading a value whose taint came from
    /// `value`.
    Read { site: Site, value: Origin },
    /// A value tainted by `taint` was stored; `cause` is why it was tainted
    /// (none: the code that made it is the addon's).
    Write {
        site: Site,
        taint: Taint,
        cause: Origin,
    },
}

/// The VM's origin records, with an index for reuse.
#[derive(Default)]
pub struct Origins {
    records: Vec<Record>,
    index: HashMap<(usize, usize, u32, bool), Origin>,
}

impl Origins {
    pub fn get(&self, o: Origin) -> Option<&Record> {
        (o.0 as usize)
            .checked_sub(1)
            .and_then(|i| self.records.get(i))
    }
}

impl Vm {
    /// Records where a read just tainted the execution, if one did.
    pub(crate) fn settle_origin(&mut self) {
        if let Some(value) = self.cur.taint.tainted_by.take() {
            self.cur.taint.origin = self.record(false, value, Taint::SECURE);
        }
    }

    /// The origin of a tainted value stored now: this write, caused by what
    /// tainted the value.
    pub(crate) fn record_write(&mut self, taint: Taint, cause: Origin) -> Origin {
        self.record(true, cause, taint)
    }

    fn record(&mut self, write: bool, link: Origin, taint: Taint) -> Origin {
        let Some(f) = self.cur.frames.last() else {
            return link;
        };
        let proto = f.proto.clone();
        let pc = f.pc.saturating_sub(1);
        let key = (Rc::as_ptr(&proto) as usize, pc, link.0, write);
        if let Some(&o) = self.origins.index.get(&key) {
            return o;
        }
        let site = self.site(&proto, pc);
        let record = if write {
            Record::Write {
                site,
                taint,
                cause: link,
            }
        } else {
            Record::Read { site, value: link }
        };
        self.origins.records.push(record);
        let o = Origin(self.origins.records.len() as u32);
        self.origins.index.insert(key, o);
        o
    }

    /// The running Lua function's site at instruction `pc`.
    fn site(&self, proto: &Proto, pc: usize) -> Site {
        let level = self.levels().into_iter().find(|l| l.what != "C");
        let func = match &level {
            Some(l) => match &l.name {
                Some((_, name)) => name.clone(),
                None if l.what == "main" => "main chunk".to_string(),
                None => format!(
                    "<{}:{}>",
                    String::from_utf8_lossy(&l.short_src),
                    l.linedefined
                ),
            },
            None => "?".to_string(),
        };
        let source = String::from_utf8_lossy(&crate::vm::chunkid(&proto.source))
            .trim_start_matches('@')
            .to_string();
        Site {
            func,
            source,
            line: proto.lines.get(pc).copied().unwrap_or(0),
            what: self.site_target(proto, pc),
        }
    }

    /// What the instruction at `pc` reads or writes.
    fn site_target(&self, p: &Proto, pc: usize) -> String {
        let key = |c: Rk| match c {
            Rk::K(k) => match p.constants[k as usize] {
                Value::Str(s) => Some(String::from_utf8_lossy(self.heap.str_bytes(s)).into_owned()),
                _ => None,
            },
            Rk::R(_) => None,
        };
        let constant = |k: u32| key(Rk::K(k)).unwrap_or_else(|| "?".to_string());
        let upval = |u: u8| {
            p.upval_names
                .get(u as usize)
                .map_or_else(|| "?".to_string(), |n| n.to_string())
        };
        match p.code.get(pc) {
            Some(Op::GetGlobal { k, .. } | Op::SetGlobal { k, .. }) => {
                format!("global '{}'", constant(*k))
            }
            Some(Op::GetTable { c, .. } | Op::SetTable { b: c, .. }) => match key(*c) {
                Some(k) => format!("field '{k}'"),
                None => "a table field".to_string(),
            },
            Some(Op::SelfOp { c, .. }) => match key(*c) {
                Some(k) => format!("method '{k}'"),
                None => "a method".to_string(),
            },
            Some(Op::GetUpval { u, .. } | Op::SetUpval { u, .. }) => {
                format!("upvalue '{}'", upval(*u))
            }
            Some(Op::Call { a, .. } | Op::TailCall { a, .. }) => {
                match crate::vm::obj_name(&self.heap, p, pc, *a) {
                    Some((kind, name)) => format!("the function in {kind} '{name}'"),
                    None => "a function".to_string(),
                }
            }
            _ => "a value".to_string(),
        }
    }

    /// The chain behind an origin, as a report's lines: each step of secure
    /// code that read a tainted value, back to the addon code that wrote it.
    pub fn taint_report(&self, start: Origin) -> String {
        let mut out = String::new();
        let mut o = start;
        let mut first = true;
        // Records are made before the ones that link to them: the chain
        // ends.
        while let Some(record) = self.origins.get(o) {
            match record {
                Record::Read { site, value } => {
                    if first {
                        out.push_str(&format!(
                            "  {}  {}:{}\n    read {}\n",
                            site.func, site.source, site.line, site.what
                        ));
                    } else {
                        out.push_str(&format!("  {}\n", site.what));
                    }
                    first = false;
                    o = *value;
                }
                Record::Write { site, taint, cause } => {
                    let by = self.taint_name(*taint).unwrap_or_default();
                    if self.origins.get(*cause).is_none() {
                        out.push_str(&format!(
                            "    written by {by}  {}:{}\n",
                            site.source, site.line
                        ));
                        break;
                    }
                    out.push_str(&format!(
                        "  written by {}\n    {}:{}\n    while tainted by {by}, after reading\n",
                        site.func, site.source, site.line
                    ));
                    first = false;
                    o = *cause;
                }
            }
        }
        out
    }

    /// The report for the running execution's taint.
    pub fn execution_report(&mut self) -> String {
        self.settle_origin();
        self.taint_report(self.cur.taint.origin)
    }

    /// The report for a stored value: `t[key]`'s taint.
    pub fn value_report(&self, t: crate::heap::TableRef, key: Value) -> String {
        let s = self.heap.table(t).get_slot(key);
        self.taint_report(s.o)
    }
}
