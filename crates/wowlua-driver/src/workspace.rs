//! Workspaces (spec §9.15, §10.8): one root manifest groups member
//! packages that share one lockfile, one `dist/` and their defaults. An
//! addon listing another workspace addon in `addon-dependencies` is checked
//! against that addon's generated declaration file, which describes its
//! `@global` API (spec compiler §13.3.1, stage 12).

use serde_json::Value;
use std::path::{Path, PathBuf};
use wowlua_syntax::view::*;

/// A member addon of the workspace, for generated declaration files.
#[derive(Clone, Debug)]
pub struct WorkspaceAddon {
    /// `addon.name` (the AddOn folder name).
    pub name: String,
    /// Its source folder.
    pub src: PathBuf,
}

/// Root keys whose values members inherit (objects merge deeply; arrays and
/// scalars of the member replace).
pub const SHARED_KEYS: &[&str] = &[
    "profiles",
    "minify-levels",
    "check",
    "fmt",
    "config",
    "publish",
];

/// `over` merged onto `base`: objects deeply, everything else replaced.
pub fn merge(base: &mut Value, over: &Value) {
    match (base, over) {
        (Value::Object(b), Value::Object(o)) => {
            for (k, v) in o {
                match b.get_mut(k) {
                    Some(slot) => merge(slot, v),
                    None => {
                        b.insert(k.clone(), v.clone());
                    }
                }
            }
        }
        (b, o) => *b = o.clone(),
    }
}

/// A member's manifest with the root's shared defaults beneath it.
pub fn with_defaults(member: &Value, root: &Value) -> Value {
    let mut merged = serde_json::Map::new();
    // A root that is a package itself publishes itself: its targets are
    // not the members' (decision T121).
    let root_is_package = root.get("package").is_some();
    for key in SHARED_KEYS {
        if root_is_package && *key == "publish" {
            continue;
        }
        if let Some(v) = root.get(*key) {
            let mut v = v.clone();
            // A root bundling its members keeps its targets for the bundle.
            if *key == "publish"
                && let Some(p) = v.as_object_mut()
                && p.get("bundle-addons")
                    .is_some_and(|b| b != &Value::Bool(false))
            {
                p.shift_remove("targets");
                p.shift_remove("bundle-addons");
            }
            merged.insert((*key).to_string(), v);
        }
    }
    let mut merged = Value::Object(merged);
    merge(&mut merged, member);
    // Keys keep the member's order, shared ones after.
    if let (Value::Object(m), Value::Object(own)) = (&merged, member) {
        let mut ordered = serde_json::Map::new();
        for k in own.keys() {
            ordered.insert(k.clone(), m[k].clone());
        }
        for (k, v) in m {
            if !ordered.contains_key(k) {
                ordered.insert(k.clone(), v.clone());
            }
        }
        return Value::Object(ordered);
    }
    merged
}

fn text_of(src: &[u8], span: wowlua_span::Span) -> String {
    let text = String::from_utf8_lossy(&src[span.start as usize..span.end as usize]);
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn is_ident(name: &str) -> bool {
    let mut chars = name.chars();
    chars
        .next()
        .is_some_and(|c| c.is_ascii_alphabetic() || c == '_')
        && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

/// The global name of a `@global` / `@global("Name")` decorator list.
fn global_name(decorators: Decorators<'_>, declared: &str) -> Option<String> {
    let d = decorators.iter().find(|d| d.is_named("global"))?;
    let name = d
        .args()
        .and_then(|a| a.exprs().first())
        .and_then(|e| match e.kind() {
            ExprKind::String(s) => Some(String::from_utf8_lossy(&s.value()).into_owned()),
            _ => None,
        })
        .unwrap_or_else(|| declared.to_string());
    is_ident(&name).then_some(name)
}

/// `declare function name(params): ret` for a function body.
fn function_decl(src: &[u8], name: &str, f: Func<'_>) -> String {
    let mut params: Vec<String> = f
        .params()
        .iter()
        .map(|p| match &p.ty() {
            Some(t) => {
                let mut t = text_of(src, t.span());
                if p.default().is_some() && !t.ends_with('?') {
                    t = if t.contains(['|', '&', ' ']) {
                        format!("({t})?")
                    } else {
                        format!("{t}?")
                    };
                }
                format!("{}: {t}", p.name().text())
            }
            None => p.name().text().to_string(),
        })
        .collect();
    if let Some(v) = &f.vararg() {
        params.push(match &v.ty {
            Some(VarargType::Type(t)) => format!("...: {}", text_of(src, t.span())),
            _ => "...".into(),
        });
    }
    let ret = match &f.ret() {
        None => String::new(),
        Some(RetType::Single(t)) => format!(": {}", text_of(src, t.span())),
        Some(RetType::List { span, .. }) => format!(": {}", text_of(src, *span)),
        Some(RetType::Pack(n)) => format!(": {}...", n.text()),
        Some(RetType::Predicate { ty, .. }) => format!(": {}", text_of(src, ty.span())),
    };
    format!("declare function {name}({}){ret}\n", params.join(", "))
}

/// The declaration of one value: its annotated type, a literal's type, or
/// `any`.
fn value_decl(src: &[u8], name: &str, ty: Option<Type<'_>>, value: Option<Expr<'_>>) -> String {
    let t = match (ty, value.map(|v| v.kind())) {
        (Some(t), _) => text_of(src, t.span()),
        (None, Some(ExprKind::Number(_))) => "number".into(),
        (None, Some(ExprKind::String(_))) => "string".into(),
        (None, Some(ExprKind::True | ExprKind::False)) => "boolean".into(),
        _ => "any".into(),
    };
    format!("declare global {name}: {t}\n")
}

/// The generated declaration file of an addon: its `@global` functions
/// (with their signatures), values (with their annotated or literal type),
/// classes and enums (as `any`).
pub fn global_declarations(src_dir: &Path) -> String {
    let mut out = String::new();
    let mut files: Vec<PathBuf> = walkdir::WalkDir::new(src_dir)
        .into_iter()
        .flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy();
            e.file_type().is_file()
                && n.ends_with(".wowlua")
                && !n.ends_with(".d.wowlua")
                && !n.ends_with(".test.wowlua")
        })
        .map(|e| e.path().to_path_buf())
        .collect();
    files.sort();
    for path in files {
        let Ok(bytes) = std::fs::read(&path) else {
            continue;
        };
        let mut db = wowlua_span::SourceDb::new();
        let file = db.add("decl", String::from_utf8_lossy(&bytes).into_owned());
        let parse = wowlua_syntax::parse(&bytes, file);
        if parse.has_errors() {
            continue;
        }
        for s in parse.view().block().stmts() {
            let inner = match s.kind() {
                StmtKind::Export(inner) => inner,
                _ => s,
            };
            let decorators = &s.decorators();
            match inner.kind() {
                StmtKind::Function { name, func }
                    if name.path().len() == 1 && name.method().is_none() =>
                {
                    if let Some(g) = global_name(*decorators, name.path().at(0).text()) {
                        out.push_str(&function_decl(&bytes, &g, func));
                    }
                }
                StmtKind::LocalFunction { name, func } => {
                    if let Some(g) = global_name(*decorators, name.text()) {
                        out.push_str(&function_decl(&bytes, &g, func));
                    }
                }
                StmtKind::Local { names, values } if names.len() == 1 => {
                    if let Some(g) = global_name(*decorators, names.at(0).name.text()) {
                        match values.first().map(|v| v.kind()) {
                            Some(ExprKind::Function(f)) if names.at(0).ty.is_none() => {
                                out.push_str(&function_decl(&bytes, &g, f));
                            }
                            _ => out.push_str(&value_decl(
                                &bytes,
                                &g,
                                names.at(0).ty,
                                values.first(),
                            )),
                        }
                    }
                }
                StmtKind::Const { name, ty, value } => {
                    if let Some(g) = global_name(*decorators, name.text()) {
                        out.push_str(&value_decl(&bytes, &g, ty, value));
                    }
                }
                StmtKind::Class(c) => {
                    if let Some(g) = global_name(*decorators, c.name().text()) {
                        out.push_str(&format!("declare global {g}: any\n"));
                    }
                }
                StmtKind::Enum(e) => {
                    if let Some(g) = global_name(*decorators, e.name().text()) {
                        out.push_str(&format!("declare global {g}: any\n"));
                    }
                }
                _ => {}
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merges_defaults_under_members() {
        let root = serde_json::json!({
            "workspace": { "members": ["a"] },
            "check": { "mode": "strict", "lua": true },
            "profiles": { "release": { "minify": "full" } },
            "publish": { "targets": { "cf": { "type": "curseforge", "project-id": 1 } } }
        });
        let member = serde_json::json!({
            "package": { "name": "a", "version": "1.0.0" },
            "check": { "lua": false },
            "publish": { "targets": { "cf": { "enabled": false } } }
        });
        let m = with_defaults(&member, &root);
        assert_eq!(
            m["check"],
            serde_json::json!({ "mode": "strict", "lua": false })
        );
        assert_eq!(m["profiles"]["release"]["minify"], "full");
        assert_eq!(m["publish"]["targets"]["cf"]["type"], "curseforge");
        assert_eq!(m["publish"]["targets"]["cf"]["enabled"], false);
        assert!(m.get("workspace").is_none());
        assert_eq!(m.as_object().unwrap().keys().next().unwrap(), "package");
    }

    #[test]
    fn generates_global_declarations() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("api.wowlua"),
            "@global\nexport function GetScore(unit: string, fallback: number = 0): number\n    return fallback\nend\n@global(\"CoreVersion\")\nexport const VERSION = \"1.0\"\n@global\nlocal function Helper(...: string) end\n@global\nexport class Tracker\nend\nlocal function hidden() end\n",
        )
        .unwrap();
        let text = global_declarations(dir.path());
        assert_eq!(
            text,
            "declare function GetScore(unit: string, fallback: number?): number\ndeclare global CoreVersion: string\ndeclare function Helper(...: string)\ndeclare global Tracker: any\n"
        );
    }
}
