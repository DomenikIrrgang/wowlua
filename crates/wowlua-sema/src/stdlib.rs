//! Compile-time checks of the core standard library (spec stdlib, "core"):
//! names given to `wowlua:events`, `wowlua:slash` and `wowlua:saved`, and
//! the deferring decorators of `wowlua:timer` and `wowlua:combat`, and the
//! keybindings of `wowlua:bindings`, collected for the generated
//! Bindings.xml. Calls and decorators are recognized through the imports'
//! `@intrinsic` ids.

use crate::constant::Const;
use crate::{Analysis, FileInput, FileKind, Target, is_compiler_known};
use std::collections::{HashMap, HashSet};
use wowlua_diagnostics::Diagnostic;
use wowlua_span::Span;
use wowlua_syntax::view::*;

/// What the checks need from the manifest.
#[derive(Clone, Debug, Default)]
pub struct StdlibConfig {
    /// `addon.saved-variables`.
    pub saved: HashSet<String>,
    /// `addon.saved-variables-per-character`.
    pub saved_per_char: HashSet<String>,
    /// Bindings of hand-written `Bindings*.xml` files (`settings.keybinding`
    /// may name them).
    pub xml_bindings: HashSet<String>,
    /// The targeted flavors' tooltips (`wowlua:tooltip` kinds), by flavor.
    pub tooltips: Vec<(String, FlavorTooltips)>,
    /// The targeted flavors' auras (`wowlua:aura`), by flavor.
    pub auras: Vec<(String, FlavorAuras)>,
}

/// What a flavor's auras offer: the filter words its client accepts
/// (`"!"` when it negates them), and whether it has aura containers.
#[derive(Clone, Debug, Default)]
pub struct FlavorAuras {
    pub words: HashSet<String>,
    pub containers: bool,
}

/// What a flavor's tooltips offer: its `Enum.TooltipDataType` names, and
/// whether it has the tooltip data system (else only `Item`, `Spell` and
/// `Unit`, through the tooltips' scripts).
#[derive(Clone, Debug, Default)]
pub struct FlavorTooltips {
    pub kinds: HashSet<String>,
    pub data_system: bool,
}

/// The media kinds of `wowlua:media` (LibSharedMedia's).
const MEDIA_KINDS: &[&str] = &["font", "statusbar", "border", "background", "sound"];

/// The kinds clients without the tooltip data system have.
const SCRIPT_KINDS: &[&str] = &["Item", "Spell", "Unit"];

/// A keybinding declared with `@binding` (spec stdlib "bindings").
#[derive(Clone, Debug)]
pub struct BindingDecl {
    pub name: String,
    pub header: Option<String>,
    pub run_on_up: bool,
    /// The default key, and where it is given.
    pub default: Option<(String, Span)>,
    pub span: Span,
}

/// What the checks found: diagnostics, and the declarations the build
/// generates files from.
#[derive(Clone, Debug, Default)]
pub struct StdlibOutput {
    pub diagnostics: Vec<Diagnostic>,
    /// `@binding` declarations, in load order.
    pub bindings: Vec<BindingDecl>,
}

/// Slash commands of the game; registering one again is W8011.
const BLIZZARD_COMMANDS: &[&str] = &[
    "/reload",
    "/console",
    "/run",
    "/script",
    "/dump",
    "/help",
    "/h",
    "/who",
    "/join",
    "/leave",
    "/invite",
    "/inv",
    "/kick",
    "/logout",
    "/exit",
    "/quit",
    "/camp",
    "/afk",
    "/dnd",
    "/say",
    "/s",
    "/yell",
    "/y",
    "/party",
    "/p",
    "/guild",
    "/g",
    "/raid",
    "/ra",
    "/whisper",
    "/w",
    "/tell",
    "/t",
    "/r",
    "/reply",
    "/emote",
    "/e",
    "/me",
    "/macro",
    "/framestack",
    "/fstack",
    "/eventtrace",
    "/etrace",
    "/target",
    "/tar",
    "/assist",
    "/a",
    "/focus",
    "/cast",
    "/use",
    "/stopmacro",
    "/click",
    "/follow",
    "/f",
    "/roll",
    "/random",
    "/rnd",
    "/played",
    "/time",
    "/friend",
    "/ignore",
    "/chat",
    "/combatlog",
    "/stopattack",
    "/startattack",
    "/dance",
    "/sit",
    "/stand",
    "/wave",
    "/bow",
    "/cheer",
    "/readycheck",
    "/rw",
    "/i",
    "/instance",
];

/// A local name bound to the standard library.
#[derive(Clone)]
enum Bound {
    Intrinsic(String),
    /// `import * as m`: the module's intrinsics by export name.
    Namespace(usize),
}

struct Ctx<'a> {
    analysis: &'a Analysis,
    events: HashSet<String>,
    messages: HashSet<String>,
    config: &'a StdlibConfig,
    diagnostics: Vec<Diagnostic>,
    slash_seen: HashMap<String, Span>,
    bound_saved: HashMap<String, Span>,
    /// The first `settings.register` call.
    settings_registered: Option<Span>,
    /// The first `launcher.register` call.
    launcher_registered: Option<Span>,
    bindings: Vec<BindingDecl>,
    /// `settings.keybinding` names, checked once every binding is known.
    keybinding_refs: Vec<(String, Span)>,
    /// The current file's top-level `const` values.
    consts: HashMap<String, Const>,
    /// The current declaration is removed by an `@if` in this build: its
    /// binding is checked, not collected.
    removed: bool,
    /// The flavor of this compilation (split mode), else all targeted ones.
    flavor: Option<String>,
    /// Statements `@if` removes in this build: calls inside are not checked.
    removed_spans: Vec<Span>,
}

/// Whether `name` is a valid keybinding name (E8352).
fn binding_name(name: &str) -> bool {
    name.starts_with(|c: char| c.is_ascii_uppercase())
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// The macro conditions a state driver understands (each also with `no`).
const MACRO_CONDITIONS: &[&str] = &[
    "actionbar",
    "advflyable",
    "bar",
    "bonusbar",
    "btn",
    "button",
    "canexitvehicle",
    "channeling",
    "combat",
    "cursor",
    "dead",
    "equipped",
    "exists",
    "extrabar",
    "flyable",
    "flying",
    "form",
    "group",
    "harm",
    "help",
    "indoors",
    "known",
    "mod",
    "modifier",
    "mounted",
    "outdoors",
    "overridebar",
    "party",
    "pet",
    "petbattle",
    "possessbar",
    "pvptalent",
    "raid",
    "resting",
    "shapeshift",
    "spec",
    "stance",
    "stealth",
    "swimming",
    "talent",
    "unithasvehicleui",
    "vehicleui",
    "worn",
];

/// Parses a macro conditional (`[combat,mod:shift] a; [@focus,help] b; c`):
/// the values of its clauses, or what is wrong with it.
fn macro_conditional(text: &str) -> Result<Vec<String>, String> {
    let mut values = Vec::new();
    for clause in text.split(';') {
        let mut rest = clause.trim_start();
        while let Some(inner) = rest.strip_prefix('[') {
            let Some(end) = inner.find(']') else {
                return Err(format!("`[` without `]` in `{}`", clause.trim()));
            };
            for cond in inner[..end].split(',').map(str::trim) {
                if cond.is_empty() {
                    continue;
                }
                if let Some(unit) = cond.strip_prefix('@') {
                    if unit.trim().is_empty() {
                        return Err("`@` without a unit".into());
                    }
                    continue;
                }
                let name = cond.split([':', '=']).next().unwrap_or("").trim();
                let lower = name.to_ascii_lowercase();
                if lower == "target" && cond.contains('=') {
                    continue;
                }
                let known = |n: &str| MACRO_CONDITIONS.contains(&n);
                if !known(&lower) && !lower.strip_prefix("no").is_some_and(known) {
                    return Err(format!("unknown condition `{name}`"));
                }
            }
            rest = inner[end + 1..].trim_start();
        }
        if rest.contains(['[', ']']) {
            return Err(format!("misplaced bracket in `{}`", clause.trim()));
        }
        values.push(rest.trim().to_string());
    }
    Ok(values)
}

/// `f` or `a.b` as text, for a call's callee.
fn call_name(callee: Expr<'_>) -> Option<String> {
    match callee.kind() {
        ExprKind::Name(n) => Some(n.text().to_string()),
        ExprKind::Field { obj, name } => match obj.kind() {
            ExprKind::Name(o) => Some(format!("{}.{}", o.text(), name.text())),
            _ => None,
        },
        _ => None,
    }
}

fn string(e: Expr<'_>) -> Option<String> {
    match e.kind() {
        ExprKind::String(lit) => Some(String::from_utf8_lossy(&lit.value()).into_owned()),
        _ => None,
    }
}

impl Ctx<'_> {
    /// A constant (a literal or folded from the file's `const`s).
    fn constant(&self, e: Expr<'_>) -> Option<Const> {
        crate::constant::eval(e, &|name| self.consts.get(name).cloned())
    }

    fn constant_string(&self, e: Expr<'_>) -> Option<String> {
        match self.constant(e)? {
            Const::String(s) => Some(String::from_utf8_lossy(&s).into_owned()),
            _ => None,
        }
    }

    fn error(&mut self, code: &'static str, span: Span, message: String, label: &str) {
        self.diagnostics
            .push(Diagnostic::error(code, message).with_label(span, label));
    }

    fn warn(&mut self, code: &'static str, span: Span, message: String, label: &str) {
        self.diagnostics
            .push(Diagnostic::warning(code, message).with_label(span, label));
    }

    /// A literal macro conditional (E8440); `visibility`: its values must
    /// be `show` or `hide` (E8441).
    fn conditional(&mut self, e: Expr<'_>, visibility: bool) {
        let Some(text) = self.constant_string(e) else {
            return;
        };
        match macro_conditional(&text) {
            Err(why) => self.error(
                "E8440",
                e.span(),
                format!("invalid macro conditional: {why}"),
                "here",
            ),
            Ok(values) if visibility => {
                let last = values.len() - 1;
                for (i, v) in values.iter().enumerate() {
                    // A trailing `;` leaves an empty last clause.
                    if v == "show" || v == "hide" || (v.is_empty() && i == last && i > 0) {
                        continue;
                    }
                    self.error(
                        "E8441",
                        e.span(),
                        format!("a visibility driver's values are `show` and `hide`, not `{v}`"),
                        "here",
                    );
                    break;
                }
            }
            Ok(_) => {}
        }
    }

    /// An event or message name: known, and of the expected kind.
    fn event_name(&mut self, e: Expr<'_>, game_only: bool) {
        let Some(name) = string(e) else { return };
        let is_event = self.events.contains(&name);
        let is_message = self.messages.contains(&name);
        if !is_event && !is_message {
            self.error(
                "E8003",
                e.span(),
                format!("unknown event or message `{name}`"),
                "not declared",
            );
        } else if game_only && !is_event {
            self.error(
                "E8003",
                e.span(),
                format!("`{name}` is a message; unit events must be WoW events"),
                "a message",
            );
        }
    }

    fn intrinsic(&self, bindings: &HashMap<String, Bound>, callee: Expr<'_>) -> Option<String> {
        match callee.kind() {
            ExprKind::Name(n) => match bindings.get(n.text())? {
                Bound::Intrinsic(id) => Some(id.clone()),
                Bound::Namespace(_) => None,
            },
            ExprKind::Field { obj, name } => match obj.kind() {
                ExprKind::Name(m) => match bindings.get(m.text())? {
                    Bound::Namespace(module) => self.analysis.modules[*module]
                        .as_ref()?
                        .intrinsics
                        .get(name.text())
                        .cloned(),
                    Bound::Intrinsic(_) => None,
                },
                _ => None,
            },
            _ => None,
        }
    }

    fn decorator(&self, bindings: &HashMap<String, Bound>, d: Decorator<'_>) -> Option<String> {
        let path = d.path();
        match path.len() {
            1 => match bindings.get(path.at(0).text())? {
                Bound::Intrinsic(id) => Some(id.clone()),
                Bound::Namespace(_) => None,
            },
            2 => match bindings.get(path.at(0).text())? {
                Bound::Namespace(module) => self.analysis.modules[*module]
                    .as_ref()?
                    .intrinsics
                    .get(path.at(1).text())
                    .cloned(),
                Bound::Intrinsic(_) => None,
            },
            _ => None,
        }
    }

    /// The decorators of a declaration; `instance_method`: an instance
    /// method of a class; `func`: the decorated function; `top`: a
    /// statement at the module's top level.
    fn decorators(
        &mut self,
        bindings: &HashMap<String, Bound>,
        decorators: Decorators<'_>,
        instance_method: bool,
        func: Option<Func<'_>>,
        top: bool,
    ) {
        for d in decorators {
            let Some(id) = self.decorator(bindings, d) else {
                continue;
            };
            let args: Exprs<'_> = d.args().map(|a| a.exprs()).unwrap_or_default();
            match id.as_str() {
                "events.event" => {
                    if instance_method {
                        self.error(
                            "E8002",
                            d.span(),
                            "`@event` on an instance method: register with `on(…, self)` in the constructor".into(),
                            "an instance method",
                        );
                    }
                    for a in &args {
                        self.event_name(a, false);
                    }
                }
                "events.bucket" => {
                    if instance_method {
                        self.error(
                            "E8002",
                            d.span(),
                            "`@bucket` on an instance method: register with `onBucket(…, self)` in the constructor".into(),
                            "an instance method",
                        );
                    }
                    for a in args.iter().skip(1) {
                        self.event_name(a, false);
                    }
                }
                "events.unitEvent" => {
                    if instance_method {
                        self.error(
                            "E8002",
                            d.span(),
                            "`@unitEvent` on an instance method: register with `onUnit(…, self)` in the constructor".into(),
                            "an instance method",
                        );
                    }
                    if let Some(first) = args.first() {
                        self.event_name(first, true);
                    }
                    let units = args.len().saturating_sub(1);
                    if !(1..=2).contains(&units) {
                        self.error(
                            "E8100",
                            d.span(),
                            format!(
                                "`@unitEvent` takes one or two units (WoW's limit), not {units}"
                            ),
                            "here",
                        );
                    }
                }
                "slash.slash" => {
                    for a in &args {
                        self.slash_command(a);
                    }
                }
                "bindings.binding" => self.binding(d, args, top && func.is_some()),
                "tooltip.onTooltip" => {
                    if let Some(first) = args.first()
                        && !self.removed
                    {
                        self.tooltip_kind(first);
                    }
                }
                "timer.throttle" | "timer.debounce" | "timer.nextFrame" | "combat.outOfCombat" => {
                    if let Some(f) = func
                        && returns_values(f.body())
                    {
                        let name = id.split('.').nth(1).unwrap_or(&id);
                        self.error(
                            "E8030",
                            d.span(),
                            format!(
                                "`@{name}` may run the function later, so it must not return values"
                            ),
                            "defers the call",
                        );
                    }
                }
                _ => {}
            }
        }
    }

    /// `@binding(name, opts)`: on a top-level function, with a literal name
    /// and constant options but the label.
    fn binding(&mut self, d: Decorator<'_>, args: Exprs<'_>, placed: bool) {
        if !placed {
            self.error(
                "E8350",
                d.span(),
                "`@binding` is allowed only on top-level function declarations".into(),
                "not a top-level function",
            );
            return;
        }
        let Some(first) = args.first() else {
            return;
        };
        let Some(name) = self.constant_string(first) else {
            self.error(
                "E8350",
                first.span(),
                "`@binding` needs the binding's name as a constant string".into(),
                "not a constant",
            );
            return;
        };
        if !binding_name(&name) {
            self.error(
                "E8352",
                first.span(),
                format!("`{name}` is no keybinding name: it matches `[A-Z][A-Z0-9_]*`"),
                "invalid",
            );
        }
        let mut decl = BindingDecl {
            name: name.clone(),
            header: None,
            run_on_up: false,
            default: None,
            span: first.span(),
        };
        match args.get(1).map(|a| a.kind()) {
            Some(ExprKind::Table(fields)) => {
                for f in fields {
                    let TableField::Named {
                        name: key, value, ..
                    } = f
                    else {
                        continue;
                    };
                    let key = key.text();
                    match key {
                        "header" | "default" => match self.constant_string(value) {
                            Some(v) if key == "header" => decl.header = Some(v),
                            Some(v) => decl.default = Some((v, value.span())),
                            None => self.error(
                                "E8350",
                                value.span(),
                                format!("`{key}` of `@binding` must be a constant string"),
                                "not a constant",
                            ),
                        },
                        "runOnUp" => match self.constant(value) {
                            Some(Const::Bool(on)) => decl.run_on_up = on,
                            _ => self.error(
                                "E8350",
                                value.span(),
                                "`runOnUp` of `@binding` must be a constant boolean".into(),
                                "not a constant",
                            ),
                        },
                        _ => {}
                    }
                }
            }
            Some(_) => {
                self.error(
                    "E8350",
                    args.at(1).span(),
                    "the options of `@binding` must be a table literal".into(),
                    "not a table literal",
                );
            }
            None => {}
        }
        if self.removed {
            return;
        }
        if let Some(first_decl) = self.bindings.iter().find(|b| b.name == name) {
            self.diagnostics.push(
                Diagnostic::error(
                    "E8351",
                    format!("the keybinding `{name}` is declared twice"),
                )
                .with_label(decl.span, "again")
                .with_label(first_decl.span, "first declared here"),
            );
            return;
        }
        self.bindings.push(decl);
    }

    /// `header(id, label)`: a top-level statement with a literal id.
    fn binding_header(&mut self, e: Expr<'_>, args: Exprs<'_>, top: bool) {
        if !top {
            self.error(
                "E8350",
                e.span(),
                "`header` is allowed only as a statement at the module's top level".into(),
                "not a top-level statement",
            );
        } else if let Some(first) = args.first()
            && self.constant_string(first).is_none()
        {
            self.error(
                "E8350",
                first.span(),
                "`header` needs the header's id as a constant string".into(),
                "not a constant",
            );
        }
    }

    /// The targeted flavors of this compilation's aura facts.
    fn aura_flavors(&self) -> Vec<&(String, FlavorAuras)> {
        self.config
            .auras
            .iter()
            .filter(|(f, _)| self.flavor.as_ref().is_none_or(|x| x == f))
            .collect()
    }

    /// A literal aura filter: every word known to every flavor of this
    /// compilation, `!` only where the flavor negates (E8310).
    fn aura_filter(&mut self, e: Expr<'_>) {
        let Some(filter) = self.constant_string(e) else {
            return;
        };
        let flavors = self.aura_flavors();
        if flavors.is_empty() {
            return;
        }
        let mut problems: Vec<String> = Vec::new();
        for raw in filter.split('|') {
            let word = raw.strip_prefix('!').unwrap_or(raw);
            if word.is_empty() {
                problems.push(format!("`{filter}` has an empty word"));
                continue;
            }
            if !self
                .config
                .auras
                .iter()
                .any(|(_, a)| a.words.contains(word))
            {
                problems.push(format!("`{word}` is not an aura filter word"));
                continue;
            }
            let lacking: Vec<&str> = flavors
                .iter()
                .filter(|(_, a)| {
                    !a.words.contains(word) || (raw.starts_with('!') && !a.words.contains("!"))
                })
                .map(|(f, _)| f.as_str())
                .collect();
            if !lacking.is_empty() {
                problems.push(format!("`{raw}` is not a filter on {}", lacking.join(", ")));
            }
        }
        for message in problems {
            self.error("E8310", e.span(), message, "in this aura filter");
        }
    }

    /// `aura.container(parent, { groups = { … } })`: the groups' filters
    /// (E8310), on flavors with aura containers (E8300).
    fn aura_container(&mut self, e: Expr<'_>, args: Exprs<'_>) {
        if self.removed {
            return;
        }
        let lacking: Vec<String> = self
            .aura_flavors()
            .iter()
            .filter(|(_, a)| !a.containers)
            .map(|(f, _)| f.clone())
            .collect();
        if !lacking.is_empty() {
            self.diagnostics.push(
                Diagnostic::error(
                    "E8300",
                    format!(
                        "`aura.container` needs aura containers, which {} lack",
                        lacking.join(", ")
                    ),
                )
                .with_label(e.span(), "not on every targeted flavor")
                .with_help(
                    "guard it with `@if(flavor == …)`, or read auras with `aura.each` there",
                ),
            );
        }
        let Some(ExprKind::Table(fields)) = args.get(1).map(|a| a.kind()) else {
            return;
        };
        for f in fields {
            let TableField::Named { name, value, .. } = f else {
                continue;
            };
            if name.text() != "groups" {
                continue;
            }
            let ExprKind::Table(groups) = value.kind() else {
                continue;
            };
            for g in groups {
                let TableField::Positional(group) = g else {
                    continue;
                };
                let ExprKind::Table(group_fields) = group.kind() else {
                    continue;
                };
                for gf in group_fields {
                    if let TableField::Named { name, value, .. } = gf
                        && name.text() == "filter"
                    {
                        self.aura_filter(value);
                    }
                }
            }
        }
    }

    /// W8430: a job's `ctx:check/yield/sleep/waitFor` inside a function
    /// handed to `pcall`, `xpcall`, `securecallfunction`, `table.sort` or
    /// `gsub`, which a coroutine cannot yield across.
    fn job_yields(&mut self, body: Block<'_>, ctx: &str) {
        let mut boundaries: Vec<Expr<'_>> = Vec::new();
        wowlua_syntax::visit::walk_block(body, &mut |e| {
            let (callee_name, args) = match e.kind() {
                ExprKind::Call { callee, args } => (call_name(callee), args),
                ExprKind::Method { name, args, .. } => (Some(name.text().to_string()), args),
                _ => return,
            };
            if matches!(
                callee_name.as_deref(),
                Some(
                    "pcall"
                        | "xpcall"
                        | "securecallfunction"
                        | "securecall"
                        | "table.sort"
                        | "sort"
                        | "string.gsub"
                        | "gsub"
                )
            ) {
                for a in args.exprs() {
                    if matches!(a.kind(), ExprKind::Function(_)) {
                        boundaries.push(a);
                    }
                }
            }
        });
        for boundary in boundaries {
            let ExprKind::Function(f) = boundary.kind() else {
                continue;
            };
            let mut yields: Vec<Span> = Vec::new();
            wowlua_syntax::visit::walk_block(f.body(), &mut |e| {
                if let ExprKind::Method { obj, name, .. } = e.kind()
                    && matches!(obj.kind(), ExprKind::Name(n) if n.text() == ctx)
                    && matches!(name.text(), "check" | "yield" | "sleep" | "waitFor")
                {
                    yields.push(e.span());
                }
            });
            for span in yields {
                self.warn(
                    "W8430",
                    span,
                    "a job cannot yield here: the coroutine would cross a pcall, sort or gsub callback".into(),
                    "yields inside a C call",
                );
            }
        }
    }

    /// A tooltip kind: known (E8330) and available on every flavor of this
    /// compilation (E8300).
    fn tooltip_kind(&mut self, e: Expr<'_>) {
        let Some(kind) = self.constant_string(e) else {
            return;
        };
        let flavors: Vec<&(String, FlavorTooltips)> = self
            .config
            .tooltips
            .iter()
            .filter(|(f, _)| self.flavor.as_ref().is_none_or(|x| x == f))
            .collect();
        if flavors.is_empty() {
            return;
        }
        if !flavors.iter().any(|(_, t)| t.kinds.contains(&kind)) {
            self.diagnostics.push(
                Diagnostic::error("E8330", format!("unknown tooltip kind `{kind}`"))
                    .with_label(e.span(), "not an `Enum.TooltipDataType` name")
                    .with_help(
                        "for example \"Item\", \"Spell\", \"Unit\", \"Mount\" or \"Currency\"",
                    ),
            );
            return;
        }
        let lacking: Vec<&str> = flavors
            .iter()
            .filter(|(_, t)| {
                if t.data_system {
                    !t.kinds.contains(&kind)
                } else {
                    !SCRIPT_KINDS.contains(&kind.as_str())
                }
            })
            .map(|(f, _)| f.as_str())
            .collect();
        if !lacking.is_empty() {
            let mut d = Diagnostic::error(
                "E8300",
                format!(
                    "`{kind}` tooltips are not available on {}",
                    lacking.join(", ")
                ),
            )
            .with_label(e.span(), "not on every targeted flavor");
            if flavors
                .iter()
                .any(|(f, t)| !t.data_system && lacking.contains(&f.as_str()))
            {
                d = d.with_help("without the tooltip data system only \"Item\", \"Spell\" and \"Unit\" tooltips are hooked; guard the hook with `@if(flavor == …)`");
            }
            self.diagnostics.push(d);
        }
    }

    fn slash_command(&mut self, e: Expr<'_>) {
        let Some(name) = string(e) else { return };
        if !name.starts_with('/') || name.len() < 2 || name.chars().any(char::is_whitespace) {
            self.error(
                "E8101",
                e.span(),
                format!("`{name}` is no slash command: it starts with `/` and has no spaces"),
                "invalid",
            );
            return;
        }
        let lower = name.to_lowercase();
        if self.slash_seen.contains_key(&lower) {
            self.error(
                "E8010",
                e.span(),
                format!("the slash command `{name}` is registered twice"),
                "again",
            );
        } else {
            self.slash_seen.insert(lower.clone(), e.span());
        }
        if BLIZZARD_COMMANDS.contains(&lower.as_str()) {
            self.warn(
                "W8011",
                e.span(),
                format!("`{name}` is one of the game's slash commands"),
                "taken by the game",
            );
        }
    }

    /// A call to the standard library; `top`: a top-level call statement.
    fn call(&mut self, bindings: &HashMap<String, Bound>, e: Expr<'_>, top: bool) {
        let ExprKind::Call { callee, args } = e.kind() else {
            return;
        };
        let Some(id) = self.intrinsic(bindings, callee) else {
            return;
        };
        let args = args.exprs();
        match id.as_str() {
            "events.on" | "events.once" => {
                if let Some(first) = args.first() {
                    self.event_name(first, false);
                }
            }
            "events.onBucket" => match args.first().map(|a| a.kind()) {
                Some(ExprKind::Table(fields)) => {
                    for field in fields {
                        if let TableField::Positional(v) = field {
                            self.event_name(v, false);
                        }
                    }
                }
                Some(_) => self.event_name(args.at(0), false),
                None => {}
            },
            "events.onUnit" => {
                if let Some(first) = args.first() {
                    self.event_name(first, true);
                }
                if let Some(ExprKind::Table(fields)) = args.get(1).map(|a| a.kind())
                    && !(1..=2).contains(&fields.len())
                {
                    self.error(
                        "E8100",
                        args.at(1).span(),
                        format!(
                            "`onUnit` takes one or two units (WoW's limit), not {}",
                            fields.len()
                        ),
                        "here",
                    );
                }
            }
            "events.send" => {
                if let Some(first) = args.first()
                    && let Some(name) = string(first)
                {
                    if self.events.contains(&name) && !self.messages.contains(&name) {
                        self.error(
                            "E8001",
                            first.span(),
                            format!("`{name}` is a WoW event; only messages can be sent"),
                            "a WoW event",
                        );
                    } else if !self.messages.contains(&name) {
                        self.error(
                            "E8003",
                            first.span(),
                            format!("unknown message `{name}`: declare it with `declare message {name}(…)`"),
                            "not declared",
                        );
                    }
                }
            }
            "secure.visibility" => {
                if let Some(c) = args.get(1) {
                    self.conditional(c, true);
                }
            }
            "secure.state" => {
                if let Some(c) = args.get(2) {
                    self.conditional(c, false);
                }
            }
            "secure.evaluate" => {
                if let Some(c) = args.first() {
                    self.conditional(c, false);
                }
            }
            "saved.bind" => self.bind(e, args),
            "bindings.header" => self.binding_header(e, args, top),
            "aura.each" | "aura.forEach" => {
                if let Some(f) = args.get(1) {
                    self.aura_filter(f);
                }
            }
            "aura.find" | "aura.findByName" => {
                if let Some(f) = args.get(2) {
                    self.aura_filter(f);
                }
            }
            "aura.container" => self.aura_container(e, args),
            "jobs.start" => {
                if let Some(ExprKind::Function(f)) = args.first().map(|a| a.kind())
                    && let Some(ctx) = f.params().first()
                {
                    self.job_yields(f.body(), ctx.name().text());
                }
            }
            "media.register" | "media.fetch" | "media.list" => {
                if let Some(first) = args.first()
                    && let Some(kind) = self.constant_string(first)
                    && !MEDIA_KINDS.contains(&kind.as_str())
                {
                    self.error(
                        "E8400",
                        first.span(),
                        format!("`{kind}` is not a media kind: \"font\", \"statusbar\", \"border\", \"background\" or \"sound\""),
                        "unknown kind",
                    );
                }
            }
            "tooltip.hook" => {
                if let Some(first) = args.first() {
                    self.tooltip_kind(first);
                }
            }
            "settings.keybinding" => {
                if let Some(first) = args.first()
                    && let Some(name) = string(first)
                {
                    self.keybinding_refs.push((name, first.span()));
                }
            }
            "settings.register" => match self.settings_registered {
                Some(first) => self.diagnostics.push(
                    Diagnostic::error("E8114", "`settings.register` is called more than once")
                        .with_label(e.span(), "again")
                        .with_label(first, "first registered here")
                        .with_help("one options page per addon: put more pages in `subcategories`"),
                ),
                None => self.settings_registered = Some(e.span()),
            },
            "comm.open" => {
                if let Some(prefix) = args.first().and_then(|a| string(a))
                    && (prefix.is_empty() || prefix.len() > 16)
                {
                    self.error(
                        "E8050",
                        args.at(0).span(),
                        format!(
                            "the addon message prefix `{prefix}` has {} characters; the client takes 1 to 16",
                            prefix.len()
                        ),
                        "this prefix",
                    );
                }
            }
            "launcher.register" => match self.launcher_registered {
                Some(first) => self.diagnostics.push(
                    Diagnostic::error("E8080", "`launcher.register` is called more than once")
                        .with_label(e.span(), "again")
                        .with_label(first, "first registered here"),
                ),
                None => self.launcher_registered = Some(e.span()),
            },
            "slash.router" => {
                if let Some(ExprKind::Table(fields)) = args.first().map(|a| a.kind()) {
                    for field in fields {
                        if let TableField::Named { value, .. } = field {
                            self.router_command(value);
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn bind(&mut self, call: Expr<'_>, args: Exprs<'_>) {
        let Some(first) = args.first() else { return };
        let Some(name) = string(first) else {
            self.error(
                "E8102",
                first.span(),
                "`saved.bind` needs the SavedVariable's name as a string literal".into(),
                "not a literal",
            );
            return;
        };
        let per_char = self.config.saved_per_char.contains(&name);
        if !self.config.saved.contains(&name) && !per_char {
            self.error(
                "E8020",
                first.span(),
                format!("`{name}` is not declared in `addon.saved-variables` or `addon.saved-variables-per-character`"),
                "not in the manifest",
            );
        }
        if let Some(previous) = self.bound_saved.get(&name).copied() {
            self.diagnostics.push(
                Diagnostic::error("E8103", format!("`{name}` is bound twice"))
                    .with_label(call.span(), "again")
                    .with_label(previous, "first bound here"),
            );
        } else {
            self.bound_saved.insert(name.clone(), call.span());
        }
        let profiles = args.get(1).is_some_and(|opts| match opts.kind() {
            ExprKind::Table(fields) => fields.iter().any(|f| {
                matches!(f, TableField::Named { name, value, .. }
                    if name.text() == "profiles" && matches!(value.kind(), ExprKind::True))
            }),
            _ => false,
        });
        if profiles {
            if per_char {
                self.error(
                    "E8117",
                    first.span(),
                    format!(
                        "`{name}` is per character: profiles need an account-wide SavedVariable"
                    ),
                    "per character",
                );
            } else if !self
                .config
                .saved_per_char
                .contains(&format!("{name}PerChar"))
            {
                self.error(
                    "E8021",
                    first.span(),
                    format!("profiles keep the active profile in `{name}PerChar`: add it to `addon.saved-variables-per-character`"),
                    "needs `PerChar`",
                );
            }
        }
    }

    /// One router command: valid `args` specs, `rest` last, and `run` taking
    /// one parameter per argument (plus the flags table).
    fn router_command(&mut self, command: Expr<'_>) {
        let ExprKind::Table(fields) = command.kind() else {
            return;
        };
        let field = |key: &str| {
            fields.iter().find_map(|f| match f {
                TableField::Named { name, value, .. } if name.text() == key => Some(value),
                _ => None,
            })
        };
        let mut count = 0;
        if let Some(ExprKind::Table(specs)) = field("args").map(|e| e.kind()) {
            count = specs.len();
            for (i, spec) in specs.iter().enumerate() {
                let TableField::Positional(value) = spec else {
                    continue;
                };
                let ok = match value.kind() {
                    ExprKind::String(lit) => {
                        let value = lit.value();
                        let s = String::from_utf8_lossy(&value);
                        matches!(
                            s.as_ref(),
                            "string" | "number" | "boolean" | "string?" | "number?" | "boolean?"
                        ) || (s == "rest" && i + 1 == specs.len())
                    }
                    ExprKind::Table(choices) => choices.iter().all(|c| {
                        matches!(c, TableField::Positional(v) if matches!(v.kind(), ExprKind::String(_)))
                    }),
                    _ => true,
                };
                if !ok {
                    self.error(
                        "E8112",
                        value.span(),
                        "an argument is \"string\", \"number\", \"boolean\" (with `?` when optional), a list of choices, or \"rest\" last".into(),
                        "invalid",
                    );
                }
            }
        }
        if field("flags").is_some() {
            count += 1;
        }
        if let Some(run) = field("run")
            && let ExprKind::Function(f) = run.kind()
            && f.vararg().is_none()
            && f.params().len() != count
        {
            self.error(
                "E8112",
                run.span(),
                format!(
                    "`run` takes {} parameter(s) for its arguments and flags, not {}",
                    count,
                    f.params().len()
                ),
                "here",
            );
        }
    }
}

/// `return` with values in the function's own body (not nested functions).
fn returns_values(block: Block<'_>) -> bool {
    fn walk(stmts: Stmts<'_>) -> bool {
        stmts.iter().any(|s| match s.kind() {
            StmtKind::Return(values) => !values.is_empty(),
            StmtKind::Do(b)
            | StmtKind::While { body: b, .. }
            | StmtKind::Repeat { body: b, .. } => walk(b.stmts()),
            StmtKind::NumericFor { body, .. } | StmtKind::GenericFor { body, .. } => {
                walk(body.stmts())
            }
            StmtKind::If {
                then_block,
                elseifs,
                else_block,
                ..
            } => {
                walk(then_block.stmts())
                    || elseifs.iter().any(|(_, b)| walk(b.stmts()))
                    || else_block.as_ref().is_some_and(|b| walk(b.stmts()))
            }
            _ => false,
        })
    }
    walk(block.stmts())
}

/// The names a file's imports bind to the standard library.
fn bindings(analysis: &Analysis, i: usize, parse: &wowlua_syntax::Parse) -> HashMap<String, Bound> {
    let mut out = HashMap::new();
    let Some(info) = analysis.modules.get(i).and_then(Option::as_ref) else {
        return out;
    };
    for s in parse.view().block().stmts() {
        let StmtKind::Import(import) = s.kind() else {
            continue;
        };
        let Some(resolved) = info.imports.iter().find(|r| r.start == s.span().start) else {
            continue;
        };
        let Target::Module(m) = resolved.target else {
            continue;
        };
        let Some(target) = analysis.modules[m].as_ref() else {
            continue;
        };
        match &import.clause() {
            ImportClause::Names(specs) => {
                for spec in specs {
                    if let Some(id) = target.intrinsics.get(spec.name.text()) {
                        let local = spec.alias.as_ref().unwrap_or(&spec.name);
                        out.insert(local.text().to_string(), Bound::Intrinsic(id.clone()));
                    }
                }
            }
            ImportClause::Namespace(n) => {
                if !target.intrinsics.is_empty() {
                    out.insert(n.text().to_string(), Bound::Namespace(m));
                }
            }
            ImportClause::SideEffect => {}
        }
    }
    out
}

/// The standard library's `@intrinsic` ids a file's imports bind, for the
/// type checker (which types handlers by their event, for example).
#[derive(Clone, Debug, Default)]
pub struct Intrinsics {
    /// `import on from "wowlua:events"`: local name → id.
    names: HashMap<String, String>,
    /// `import * as events from "wowlua:events"`: local name → export → id.
    namespaces: HashMap<String, HashMap<String, String>>,
}

impl Intrinsics {
    pub fn of(analysis: &Analysis, file: usize, parse: &wowlua_syntax::Parse) -> Intrinsics {
        let mut out = Intrinsics::default();
        for (local, bound) in bindings(analysis, file, parse) {
            match bound {
                Bound::Intrinsic(id) => {
                    out.names.insert(local, id);
                }
                Bound::Namespace(m) => {
                    if let Some(info) = analysis.modules[m].as_ref() {
                        out.namespaces.insert(local, info.intrinsics.clone());
                    }
                }
            }
        }
        out
    }

    /// The id `name` (or `namespace.member`) is bound to.
    pub fn get(&self, name: &str, member: Option<&str>) -> Option<&str> {
        match member {
            None => self.names.get(name),
            Some(m) => self.namespaces.get(name)?.get(m),
        }
        .map(String::as_str)
    }

    /// The local names bound (to shadowing checks).
    pub fn locals(&self) -> impl Iterator<Item = &str> {
        self.names
            .keys()
            .chain(self.namespaces.keys())
            .map(String::as_str)
    }
}

/// Runs the checks over every file of the package; `meta`: the build's
/// (for `@if`), where a file has none of its own.
pub fn check(
    files: &[FileInput],
    analysis: &Analysis,
    config: &StdlibConfig,
    meta: Option<&crate::meta::Meta>,
) -> StdlibOutput {
    let mut ctx = Ctx {
        analysis,
        events: HashSet::new(),
        messages: HashSet::new(),
        config,
        diagnostics: Vec::new(),
        slash_seen: HashMap::new(),
        bound_saved: HashMap::new(),
        settings_registered: None,
        launcher_registered: None,
        bindings: Vec::new(),
        keybinding_refs: Vec::new(),
        consts: HashMap::new(),
        removed: false,
        flavor: meta.and_then(|m| m.flavor.clone()),
        removed_spans: Vec::new(),
    };
    for f in files {
        let Some(parse) = f.parse else { continue };
        for s in parse.view().block().stmts() {
            let decl = match s.kind() {
                StmtKind::Declare(d) => d,
                StmtKind::Export(inner) => match inner.kind() {
                    StmtKind::Declare(d) => d,
                    _ => continue,
                },
                _ => continue,
            };
            match decl.kind() {
                DeclareKind::Event { name, .. } => {
                    ctx.events.insert(name.text().to_string());
                }
                DeclareKind::Message { name, .. } => {
                    ctx.messages.insert(name.text().to_string());
                }
                _ => {}
            }
        }
    }
    for (file, f) in files.iter().enumerate() {
        if f.kind != FileKind::Wowlua || is_compiler_known(&f.src_rel) {
            continue;
        }
        let Some(parse) = f.parse else { continue };
        let bound = bindings(analysis, file, parse);
        if bound.is_empty() {
            continue;
        }
        ctx.consts.clear();
        for s in parse.view().block().stmts() {
            let inner = match s.kind() {
                StmtKind::Export(inner) => inner,
                _ => s,
            };
            if let StmtKind::Const {
                name,
                value: Some(value),
                ..
            } = &inner.kind()
                && let Some(c) = ctx.constant(*value)
            {
                ctx.consts.insert(name.text().to_string(), c);
            }
        }
        let top: HashSet<Stmt<'_>> = parse.view().block().stmts().iter().collect();
        let mut stmts: Vec<Stmt<'_>> = Vec::new();
        wowlua_syntax::visit::walk_stmts(parse.view().block(), &mut |s| stmts.push(s));
        let mut i = 0;
        while i < stmts.len() {
            let s = stmts[i];
            i += 1;
            let inner = match s.kind() {
                StmtKind::Export(inner) => inner,
                _ => s,
            };
            let at_top = top.contains(&s);
            ctx.removed = f.meta.or(meta).is_some_and(|meta| {
                let kind_of = |n: &str| {
                    analysis.modules[file]
                        .as_ref()
                        .and_then(|m| m.meta_bindings.get(n).copied())
                };
                s.decorators().iter().any(|d| {
                    crate::meta::if_condition(d)
                        .flatten()
                        .and_then(|cond| {
                            crate::meta::static_value(cond, meta, &kind_of, &|n| {
                                ctx.consts.get(n).cloned()
                            })
                        })
                        .is_some_and(|c| !c.truthy())
                })
            });
            if ctx.removed {
                ctx.removed_spans.push(s.span());
            }
            match inner.kind() {
                StmtKind::LocalFunction { func, .. } | StmtKind::Function { func, .. } => {
                    ctx.decorators(&bound, s.decorators(), false, Some(func), at_top);
                }
                StmtKind::Class(c) => {
                    for m in c.members() {
                        let static_ = m.modifiers().contains(&Modifier::Static);
                        if let ClassMemberKind::Method { func, .. } = m.kind() {
                            ctx.decorators(&bound, m.decorators(), !static_, Some(func), false);
                            wowlua_syntax::visit::walk_stmts(func.body(), &mut |s| stmts.push(s));
                        }
                    }
                }
                _ => ctx.decorators(&bound, s.decorators(), false, None, at_top),
            }
        }
        let mut calls: Vec<Expr<'_>> = Vec::new();
        wowlua_syntax::visit::walk_block(parse.view().block(), &mut |e| {
            if matches!(e.kind(), ExprKind::Call { .. }) {
                calls.push(e);
            }
        });
        for s in &stmts {
            if let StmtKind::Class(c) = s.kind() {
                for m in c.members() {
                    if let ClassMemberKind::Method { func, .. } = m.kind() {
                        wowlua_syntax::visit::walk_block(func.body(), &mut |e| {
                            if matches!(e.kind(), ExprKind::Call { .. }) {
                                calls.push(e);
                            }
                        });
                    }
                }
            }
        }
        let top_calls: HashSet<Expr<'_>> = parse
            .view()
            .block()
            .stmts()
            .iter()
            .filter_map(|s| match s.kind() {
                StmtKind::Call(e) => Some(e),
                _ => None,
            })
            .collect();
        for e in calls {
            let removed = ctx.removed_spans.iter().any(|r| {
                r.file == e.span().file && r.start <= e.span().start && e.span().end <= r.end
            });
            if !removed {
                ctx.call(&bound, e, top_calls.contains(&e));
            }
        }
        ctx.removed_spans.clear();
    }
    let refs = std::mem::take(&mut ctx.keybinding_refs);
    for (name, span) in refs {
        if !ctx.bindings.iter().any(|b| b.name == name) && !config.xml_bindings.contains(&name) {
            ctx.error(
                "E8353",
                span,
                format!("`{name}` is not a keybinding of this addon: declare it with `@binding`"),
                "not declared",
            );
        }
    }
    StdlibOutput {
        diagnostics: ctx.diagnostics,
        bindings: ctx.bindings,
    }
}
