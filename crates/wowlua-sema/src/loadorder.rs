//! Load-order analysis (spec §5.4, compiler §13.6.1): dependency edges from
//! imports and usage analysis, classified load-time or run-time; cycles
//! (E1014, or lazy bindings for run-time-only cycles); ambiguity (W1033),
//! late definitions (W1034) and usage edges that contradict
//! `build.load-order` (W1035: the listed order wins, decision T70).

use crate::usage::{CallTarget, FileUsage, Key, Markers, Path, file_usage, xml_file_usage};
use crate::{Analysis, FileInput, FileKind, Target};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use wowlua_diagnostics::Diagnostic;
use wowlua_span::Span;
use wowlua_syntax::ast::{ImportClause, StmtKind};

#[derive(Debug, Default)]
pub struct LoadOrder {
    /// `(a, b)`: file `a` must load before file `b`.
    pub constraints: Vec<(usize, usize)>,
    /// Why each constraint exists: the use and a description.
    pub reasons: HashMap<(usize, usize), (Span, String)>,
    /// Imports read through their module table: (file, import start).
    pub lazy: HashSet<(usize, u32)>,
    pub diagnostics: Vec<Diagnostic>,
}

/// Why an edge exists: the use and the call chain that runs it.
#[derive(Clone, Debug)]
struct Reason {
    what: String,
    span: Span,
    chain: String,
}

#[derive(Clone, Debug)]
struct Edge {
    from: usize,
    to: usize,
    load_time: bool,
    /// The import statement, for import edges.
    import: Option<u32>,
    /// Found by usage analysis (not an `import` statement).
    usage: bool,
    reason: Reason,
}

/// Where files load, as far as the package layout fixes it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Layout<'a> {
    /// The position at which a file loads by `build.load-order` (the listed
    /// order resolves ambiguity and wins over usage edges).
    pub listed: &'a [Option<usize>],
    /// The file is part of this flavor's build.
    pub active: &'a [bool],
    /// The XML file that loads a file (`<Script>`/`<Include>`).
    pub loaded_by: &'a [Option<usize>],
}

/// Analyzes the load order.
pub fn analyze(
    files: &[FileInput],
    analysis: &Analysis,
    layout: Layout,
    markers: &Markers,
) -> LoadOrder {
    let Layout {
        listed,
        active,
        loaded_by,
    } = layout;
    // An XML file and the files it loads interleave in document order.
    let encloses = |outer: usize, mut inner: usize| {
        for _ in 0..64 {
            match loaded_by.get(inner).copied().flatten() {
                Some(p) if p == outer => return true,
                Some(p) => inner = p,
                None => return false,
            }
        }
        false
    };
    let is_active = |i: usize| active.get(i).copied().unwrap_or(true);
    let ordered = |i: usize| listed.get(i).copied().flatten().is_some();
    let usages: Vec<Option<FileUsage>> = files
        .iter()
        .enumerate()
        .map(|(i, f)| match (f.kind, f.parse) {
            (FileKind::Wowlua | FileKind::Lua, Some(p)) if is_active(i) => {
                Some(file_usage(p, markers))
            }
            (FileKind::Xml, _) if is_active(i) => f
                .xml
                .map(|(text, file)| xml_file_usage(text, file, markers)),
            _ => None,
        })
        .collect();

    // Functions stored under global or `ns` paths, package-wide.
    let mut function_index: HashMap<Path, Vec<(usize, usize)>> = HashMap::new();
    for (file, u) in usages.iter().enumerate() {
        for (path, f) in u.iter().flat_map(|u| &u.function_defs) {
            function_index
                .entry(path.clone())
                .or_default()
                .push((file, *f));
        }
    }

    // Imported bindings: (file, binding) → (target file, export name).
    let mut bindings: HashMap<(usize, String), (usize, Option<String>)> = HashMap::new();
    for (file, f) in files.iter().enumerate() {
        let (Some(parse), Some(info)) = (f.parse, &analysis.modules[file]) else {
            continue;
        };
        for s in &parse.chunk.block.stmts {
            let StmtKind::Import(import) = &s.kind else {
                continue;
            };
            let Some(resolved) = info.imports.iter().find(|i| i.start == s.span.start) else {
                continue;
            };
            let target = match resolved.target {
                Target::Module(m) => m,
                Target::Lua { file: l, .. } => l,
                _ => continue,
            };
            match &import.clause {
                ImportClause::Names(specs) => {
                    for spec in specs {
                        let local = spec.alias.as_ref().unwrap_or(&spec.name);
                        bindings.insert(
                            (file, local.text.to_string()),
                            (target, Some(spec.name.text.to_string())),
                        );
                    }
                }
                ImportClause::Namespace(n) => {
                    bindings.insert((file, n.text.to_string()), (target, None));
                }
                ImportClause::SideEffect => {}
            }
        }
    }

    let resolve = |file: usize, target: &CallTarget| -> Vec<(usize, usize)> {
        match target {
            CallTarget::Local(f) => vec![(file, *f)],
            CallTarget::Path(p) => function_index.get(p).cloned().unwrap_or_default(),
            CallTarget::Import { binding, fields } => {
                let Some((target_file, export)) = bindings.get(&(file, binding.clone())) else {
                    return Vec::new();
                };
                let name = match (export, fields.first()) {
                    (Some(e), None) => e.clone(),
                    (None, Some(f)) => f.clone(),
                    _ => return Vec::new(),
                };
                usages[*target_file]
                    .as_ref()
                    .and_then(|u| u.exported_functions.get(&name))
                    .map(|f| vec![(*target_file, *f)])
                    .unwrap_or_default()
            }
            CallTarget::Param(_) | CallTarget::Unknown => Vec::new(),
        }
    };

    // Parameters each function calls, directly or by passing them on to a
    // callee that calls them (or to one the analysis cannot resolve).
    let mut called_params: HashMap<(usize, usize), HashSet<usize>> = HashMap::new();
    for (file, u) in usages.iter().enumerate() {
        for (f, function) in u.iter().flat_map(|u| u.functions.iter().enumerate()) {
            if !function.called_params.is_empty() {
                called_params.insert((file, f), function.called_params.clone());
            }
        }
    }
    let calls_arg = |called_params: &HashMap<(usize, usize), HashSet<usize>>,
                     targets: &[(usize, usize)],
                     pos: usize| {
        targets.is_empty()
            || targets
                .iter()
                .any(|t| called_params.get(t).is_some_and(|s| s.contains(&pos)))
    };
    loop {
        let mut added = Vec::new();
        for (file, u) in usages.iter().enumerate() {
            for (f, function) in u.iter().flat_map(|u| u.functions.iter().enumerate()) {
                for call in function.calls.iter().filter(|c| !c.deferred) {
                    let targets = resolve(file, &call.target);
                    for (pos, arg) in &call.args {
                        if let CallTarget::Param(p) = arg
                            && !called_params.get(&(file, f)).is_some_and(|s| s.contains(p))
                            && calls_arg(&called_params, &targets, *pos)
                        {
                            added.push(((file, f), *p));
                        }
                    }
                }
            }
        }
        if added.is_empty() {
            break;
        }
        for (key, p) in added {
            called_params.entry(key).or_default().insert(p);
        }
    }

    // For every file R: the functions that run while R loads (its top-level
    // code and everything called from there, across files), with the call
    // that reaches each one.
    type Parents = HashMap<(usize, usize), Option<(usize, usize, Span)>>;
    let mut exec: Vec<Parents> = vec![HashMap::new(); files.len()];
    for (root, u) in usages.iter().enumerate() {
        if u.is_none() {
            continue;
        }
        let parents = &mut exec[root];
        parents.insert((root, 0), None);
        let mut work = vec![(root, 0)];
        while let Some((file, f)) = work.pop() {
            let Some(function) = usages[file].as_ref().and_then(|u| u.functions.get(f)) else {
                continue;
            };
            for call in &function.calls {
                let targets = resolve(file, &call.target);
                let mut called = targets.clone();
                if !call.deferred {
                    for (pos, arg) in &call.args {
                        if calls_arg(&called_params, &targets, *pos) {
                            called.extend(resolve(file, arg));
                        }
                    }
                }
                for target in called {
                    if let std::collections::hash_map::Entry::Vacant(slot) = parents.entry(target) {
                        slot.insert(Some((file, f, call.span)));
                        work.push(target);
                    }
                }
            }
        }
    }
    let runs_at_load = |root: usize, file: usize, f: usize| exec[root].contains_key(&(file, f));

    // Definitions: a file defines a key at load time when the definition
    // runs while it loads (also inside functions of other files it calls).
    let mut definers: BTreeMap<Key, Vec<(usize, bool, Span)>> = BTreeMap::new();
    let mut add_def = |key: &Key, file: usize, lt: bool, span: Span| {
        let list = definers.entry(key.clone()).or_default();
        match list.iter_mut().find(|(f, _, _)| *f == file) {
            Some(entry) => entry.1 |= lt,
            None => list.push((file, lt, span)),
        }
    };
    for (file, u) in usages.iter().enumerate() {
        for d in u.iter().flat_map(|u| &u.defs) {
            add_def(&d.key, file, runs_at_load(file, file, d.function), d.span);
        }
    }
    for (root, parents) in exec.iter().enumerate() {
        for &(file, f) in parents.keys() {
            if file == root {
                continue;
            }
            for d in usages[file]
                .iter()
                .flat_map(|u| &u.defs)
                .filter(|d| d.function == f)
            {
                add_def(&d.key, root, true, d.span);
            }
        }
    }

    let mut out = LoadOrder::default();
    let mut edges: Vec<Edge> = Vec::new();
    let mut warned: BTreeSet<String> = BTreeSet::new();
    let chain_of = |root: usize, file: usize, f: usize| -> String {
        let name = |file: usize, f: usize| {
            usages[file]
                .as_ref()
                .and_then(|u| u.functions.get(f))
                .map(|func| func.name.clone())
                .unwrap_or_default()
        };
        let mut parts = vec![format!("in {}", name(file, f))];
        let mut current = (file, f);
        for _ in 0..16 {
            match exec[root].get(&current) {
                Some(Some((pf, pfn, _))) => {
                    parts.push(format!("called from {}", name(*pf, *pfn)));
                    current = (*pf, *pfn);
                }
                _ => break,
            }
        }
        if root != file {
            parts.push(format!("while {} loads", files[root].src_rel));
        }
        parts.join(", ")
    };

    // Uses at load time order their definer before the file being loaded.
    for (root, parents) in exec.iter().enumerate() {
        let mut seen: HashSet<Key> = HashSet::new();
        let mut visited: Vec<&(usize, usize)> = parents.keys().collect();
        visited.sort();
        for &(file, f) in visited {
            for access in usages[file]
                .iter()
                .flat_map(|u| &u.uses)
                .filter(|a| a.function == f && !a.guarded)
            {
                if !seen.insert(access.key.clone()) {
                    continue;
                }
                let Some(defs) = definers.get(&access.key) else {
                    continue;
                };
                let lt_defs: Vec<&(usize, bool, Span)> = defs.iter().filter(|d| d.1).collect();
                let reason = Reason {
                    what: format!("uses `{}`", access.key.display()),
                    span: access.span,
                    chain: chain_of(root, file, f),
                };
                match lt_defs.as_slice() {
                    [only] if only.0 != root => edges.push(Edge {
                        from: only.0,
                        to: root,
                        load_time: true,
                        import: None,
                        usage: true,
                        reason,
                    }),
                    [_] => {}
                    [] => {
                        if warned.insert(format!("late {} {root}", access.key.display())) {
                            out.diagnostics.push(
                                Diagnostic::warning(
                                    "W1034",
                                    format!(
                                        "`{}` is read while the file loads, but only defined later (inside functions that run at run time)",
                                        access.key.display()
                                    ),
                                )
                                .with_label(access.span, "read at load time"),
                            );
                        }
                    }
                    several => {
                        // Several files define the name at load time: their
                        // order is ambiguous unless `build.load-order` fixes it.
                        let unordered = several.iter().any(|d| !ordered(d.0));
                        if unordered && warned.insert(format!("ambiguous {}", access.key.display()))
                        {
                            let mut d = Diagnostic::warning(
                                "W1033",
                                format!(
                                    "the load order of the files defining `{}` cannot be determined; they keep their source order",
                                    access.key.display()
                                ),
                            )
                            .with_help("import the module that should load first, or list the files in `build.load-order`");
                            for def in several {
                                d = d.with_label(def.2, "defined here at load time");
                            }
                            out.diagnostics.push(d);
                        }
                    }
                }
            }
        }
    }
    for (file, u) in usages.iter().enumerate() {
        if let Some(span) = u.as_ref().and_then(|u| u.dynamic_write)
            && !ordered(file)
        {
            out.diagnostics.push(
                Diagnostic::warning(
                    "W1033",
                    "a write through a dynamic name makes the load order of the names it defines undecidable",
                )
                .with_label(span, "dynamic name"),
            );
        }
    }

    // Import edges: load-time when a binding is used while the importer
    // loads; a use while another file loads orders the target before that
    // file (the binding is then read through the module table if needed).
    for (file, info) in analysis.modules.iter().enumerate() {
        let Some(info) = info else { continue };
        let Some(parse) = files[file].parse else {
            continue;
        };
        if !is_active(file) {
            continue;
        }
        for import in info.imports.iter().filter(|i| !i.type_only) {
            let target = match import.target {
                Target::Module(m) | Target::Xml(m) => m,
                Target::Lua { file: l, .. } => l,
                Target::Meta => continue,
            };
            if !is_active(target) {
                continue;
            }
            let Some(stmt) = parse
                .chunk
                .block
                .stmts
                .iter()
                .find(|s| s.span.start == import.start)
            else {
                continue;
            };
            let names: Vec<String> = match &stmt.kind {
                StmtKind::Import(i) => match &i.clause {
                    ImportClause::Names(specs) => specs
                        .iter()
                        .map(|s| s.alias.as_ref().unwrap_or(&s.name).text.to_string())
                        .collect(),
                    ImportClause::Namespace(n) => vec![n.text.to_string()],
                    ImportClause::SideEffect => Vec::new(),
                },
                _ => Vec::new(),
            };
            let uses: Vec<(String, usize, Span)> = usages[file]
                .iter()
                .flat_map(|u| &u.import_uses)
                .filter(|(b, _, _)| names.contains(b))
                .cloned()
                .collect();
            let own = uses.iter().find(|(_, f, _)| runs_at_load(file, file, *f));
            let (load_time, reason) = match (own, names.is_empty()) {
                (Some((b, f, span)), _) => (
                    true,
                    Reason {
                        what: format!("uses the import `{b}`"),
                        span: *span,
                        chain: chain_of(file, file, *f),
                    },
                ),
                (None, true) => (
                    true,
                    Reason {
                        what: "imports it with `import _`".into(),
                        span: stmt.span,
                        chain: "top-level code".into(),
                    },
                ),
                (None, false) => (
                    false,
                    Reason {
                        what: "imports it".into(),
                        span: stmt.span,
                        chain: String::new(),
                    },
                ),
            };
            edges.push(Edge {
                from: target,
                to: file,
                load_time,
                import: Some(import.start),
                usage: false,
                reason,
            });
            for (root, _) in exec
                .iter()
                .enumerate()
                .filter(|(r, _)| *r != file && *r != target)
            {
                if let Some((b, f, span)) =
                    uses.iter().find(|(_, f, _)| runs_at_load(root, file, *f))
                {
                    edges.push(Edge {
                        from: target,
                        to: root,
                        load_time: true,
                        import: None,
                        usage: true,
                        reason: Reason {
                            what: format!("uses the import `{b}` of {}", files[file].src_rel),
                            span: *span,
                            chain: chain_of(root, file, *f),
                        },
                    });
                }
            }
        }
    }

    let describe = |e: &Edge| {
        let mut text = format!(
            "`{}` {} from `{}`",
            files[e.to].src_rel, e.reason.what, files[e.from].src_rel
        );
        if e.load_time {
            text.push_str(" while it loads");
        }
        if !e.reason.chain.is_empty() && e.reason.chain != "in top-level code" {
            text.push_str(&format!(" ({})", e.reason.chain));
        }
        text
    };

    edges.retain(|e| !e.usage || !(encloses(e.from, e.to) || encloses(e.to, e.from)));
    // Usage edges that contradict `build.load-order` give way to it (W1035).
    let mut contradicted: BTreeSet<(usize, usize)> = BTreeSet::new();
    edges.retain(|e| {
        let (Some(Some(a)), Some(Some(b))) = (listed.get(e.from), listed.get(e.to)) else {
            return true;
        };
        if !e.usage || a <= b {
            return true;
        }
        if contradicted.insert((e.from, e.to)) {
            out.diagnostics.push(
                Diagnostic::warning(
                    "W1035",
                    format!(
                        "`build.load-order` loads `{}` before `{}`, which uses a name it defines; keeping the listed order",
                        files[e.to].src_rel, files[e.from].src_rel
                    ),
                )
                .with_label(e.reason.span, describe(e))
                .with_help("if the listed order is wrong, change `build.load-order`; otherwise move the use into a function that runs later"),
            );
        }
        false
    });

    // Load-time edges must not form a cycle (E1014, decision: mixed cycles
    // with run-time edges are allowed).
    let n = files.len();
    let lt_edges: Vec<Edge> = edges.iter().filter(|e| e.load_time).cloned().collect();
    let lt_sccs = strongly_connected(n, &lt_edges);
    let mut lt_cycle = vec![usize::MAX; n];
    for (c, members) in lt_sccs.iter().enumerate().filter(|(_, m)| m.len() > 1) {
        for &m in members {
            lt_cycle[m] = c;
        }
    }
    for members in lt_sccs.iter().filter(|m| m.len() > 1) {
        let set: HashSet<usize> = members.iter().copied().collect();
        let names: Vec<&str> = members.iter().map(|&m| files[m].src_rel.as_str()).collect();
        let mut d = Diagnostic::error(
            "E1014",
            format!("load-time dependency cycle between {}", names.join(", ")),
        )
        .with_help("move the use into a function that runs later (an event handler, for example), or split a module");
        for e in lt_edges
            .iter()
            .filter(|e| set.contains(&e.from) && set.contains(&e.to))
        {
            d = d
                .with_label(
                    e.reason.span,
                    format!(
                        "`{}` {} from `{}` while it loads",
                        files[e.to].src_rel, e.reason.what, files[e.from].src_rel
                    ),
                )
                .with_note(format!("{}: {}", files[e.to].src_rel, e.reason.chain));
        }
        out.diagnostics.push(d);
    }
    // Run-time imports constrain the order where they can; inside a cycle
    // they become lazy bindings.
    let sccs = strongly_connected(n, &edges);
    let mut component = vec![usize::MAX; n];
    for (c, members) in sccs.iter().enumerate() {
        for &m in members {
            component[m] = c;
        }
    }
    for e in &edges {
        let in_cycle = component[e.from] == component[e.to] && sccs[component[e.from]].len() > 1;
        if e.load_time {
            // A cycle reported as E1014 does not constrain the order.
            if lt_cycle[e.from] == usize::MAX || lt_cycle[e.from] != lt_cycle[e.to] {
                out.constraints.push((e.from, e.to));
                out.reasons
                    .entry((e.from, e.to))
                    .or_insert_with(|| (e.reason.span, describe(e)));
            }
        } else if let Some(start) = e.import {
            if in_cycle {
                out.lazy.insert((e.to, start));
            } else {
                out.constraints.push((e.from, e.to));
                out.reasons
                    .entry((e.from, e.to))
                    .or_insert_with(|| (e.reason.span, describe(e)));
            }
        }
    }
    out.constraints.sort();
    out.constraints.dedup();
    out
}

/// The strongly connected components of the file graph.
fn strongly_connected(n: usize, edges: &[Edge]) -> Vec<Vec<usize>> {
    let mut next: Vec<Vec<usize>> = vec![Vec::new(); n];
    for e in edges {
        next[e.from].push(e.to);
    }
    // Iterative Tarjan.
    let mut index = vec![usize::MAX; n];
    let mut low = vec![0usize; n];
    let mut on_stack = vec![false; n];
    let mut stack: Vec<usize> = Vec::new();
    let mut counter = 0;
    let mut out = Vec::new();
    for root in 0..n {
        if index[root] != usize::MAX {
            continue;
        }
        let mut call: Vec<(usize, usize)> = vec![(root, 0)];
        index[root] = counter;
        low[root] = counter;
        counter += 1;
        stack.push(root);
        on_stack[root] = true;
        while let Some(&mut (v, ref mut i)) = call.last_mut() {
            if *i < next[v].len() {
                let w = next[v][*i];
                *i += 1;
                if index[w] == usize::MAX {
                    index[w] = counter;
                    low[w] = counter;
                    counter += 1;
                    stack.push(w);
                    on_stack[w] = true;
                    call.push((w, 0));
                } else if on_stack[w] {
                    low[v] = low[v].min(index[w]);
                }
            } else {
                call.pop();
                if let Some(&(parent, _)) = call.last() {
                    low[parent] = low[parent].min(low[v]);
                }
                if low[v] == index[v] {
                    let mut members = Vec::new();
                    while let Some(w) = stack.pop() {
                        on_stack[w] = false;
                        members.push(w);
                        if w == v {
                            break;
                        }
                    }
                    members.sort();
                    out.push(members);
                }
            }
        }
    }
    out
}
