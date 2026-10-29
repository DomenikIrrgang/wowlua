//! Bundling (spec optimization §15.3, codegen §14.8.2): the Lua files of
//! the TOC become as few files as the load order allows. Runs of bundlable
//! files between files that must stay separate (XML, locale files, files of
//! other AddOns, libraries kept apart) become one segment each:
//! `<Name>.lua`, `<Name>.2.lua`, …, or `<Name>.<flavor>[.<n>].lua` when the
//! flavors load different files. Each file runs in a function of its own,
//! protected, so a load error stops only that file, as in separate files.

use std::collections::HashMap;
use std::sync::Arc;
use wowlua_luair::sourcemap::{EncodedMap, Segment, SegmentEncoder, SourceMap};

/// What becomes of the TOC's entries.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Planned {
    /// An entry that stays a file of its own (its index).
    Keep(usize),
    /// A bundle of the entries `members`, in load order.
    Bundle {
        path: String,
        flavors: Option<Vec<String>>,
        members: Vec<usize>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum Run {
    Keep(usize),
    Bundle(Vec<usize>),
}

/// Plans the bundles. `entries`: per TOC entry, whether it can be bundled
/// and its flavor restriction; `orders`: per-flavor orders of the entries
/// (per-flavor TOC files), which the result keeps.
pub fn plan(
    addon: &str,
    entries: &[(bool, Option<Vec<String>>)],
    flavors: &[String],
    orders: Option<&HashMap<String, Vec<usize>>>,
) -> (Vec<Planned>, Option<HashMap<String, Vec<usize>>>) {
    let sequence = |f: &str| -> Vec<usize> {
        match orders.and_then(|o| o.get(f)) {
            Some(order) => order.clone(),
            None => (0..entries.len())
                .filter(|&i| {
                    entries[i]
                        .1
                        .as_ref()
                        .is_none_or(|l| l.iter().any(|x| x == f))
                })
                .collect(),
        }
    };
    let runs = |seq: Vec<usize>| -> Vec<Run> {
        let mut out = Vec::new();
        let mut current: Vec<usize> = Vec::new();
        for i in seq {
            if entries[i].0 {
                current.push(i);
            } else {
                if !current.is_empty() {
                    out.push(Run::Bundle(std::mem::take(&mut current)));
                }
                out.push(Run::Keep(i));
            }
        }
        if !current.is_empty() {
            out.push(Run::Bundle(current));
        }
        out
    };
    let per_flavor: Vec<(String, Vec<Run>)> = flavors
        .iter()
        .map(|f| (f.clone(), runs(sequence(f))))
        .collect();
    let shared = per_flavor.windows(2).all(|w| w[0].1 == w[1].1);
    let name = |flavor: Option<&str>, n: usize| -> String {
        let mut name = addon.to_string();
        if let Some(f) = flavor {
            name.push('.');
            name.push_str(f);
        }
        if n > 1 {
            name.push_str(&format!(".{n}"));
        }
        name.push_str(".lua");
        name
    };
    // One TOC list: files every flavor loads form shared bundles; the
    // stretches of flavor-specific files between them, per-flavor bundles.
    if orders.is_none() {
        return (plan_shared(&name, entries, flavors), None);
    }
    if shared {
        let runs = per_flavor
            .first()
            .map(|(_, r)| r.clone())
            .unwrap_or_else(|| runs((0..entries.len()).collect()));
        let mut planned = Vec::new();
        let mut n = 0;
        for run in runs {
            planned.push(match run {
                Run::Keep(i) => Planned::Keep(i),
                Run::Bundle(members) => {
                    n += 1;
                    Planned::Bundle {
                        path: name(None, n),
                        flavors: None,
                        members,
                    }
                }
            });
        }
        let orders = orders.map(|o| {
            o.keys()
                .map(|f| (f.clone(), (0..planned.len()).collect()))
                .collect()
        });
        return (planned, orders);
    }
    // Per-flavor bundles, but a segment every flavor has alike is shared,
    // as are the entries kept apart.
    let mut alike: HashMap<Vec<usize>, usize> = HashMap::new();
    for (_, runs) in &per_flavor {
        for run in runs {
            if let Run::Bundle(members) = run {
                *alike.entry(members.clone()).or_default() += 1;
            }
        }
    }
    let shared_runs: std::collections::HashSet<Vec<usize>> = alike
        .into_iter()
        .filter(|(_, n)| *n == per_flavor.len())
        .map(|(m, _)| m)
        .collect();
    let mut planned: Vec<(usize, usize, Planned)> = Vec::new();
    let mut kept: HashMap<usize, usize> = HashMap::new();
    let mut shared_slots: HashMap<Vec<usize>, usize> = HashMap::new();
    // Shared bundles are numbered in load order.
    let mut shared_order: Vec<Vec<usize>> = shared_runs.iter().cloned().collect();
    shared_order.sort_by_key(|m| m[0]);
    let shared_names: HashMap<Vec<usize>, String> = shared_order
        .iter()
        .enumerate()
        .map(|(i, m)| (m.clone(), name(None, i + 1)))
        .collect();
    let mut flavor_runs: Vec<(String, Vec<usize>)> = Vec::new();
    for (fi, (flavor, runs)) in per_flavor.into_iter().enumerate() {
        let mut n = 0;
        let mut seq = Vec::new();
        for run in runs {
            match run {
                Run::Keep(i) => {
                    let slot = *kept.entry(i).or_insert_with(|| {
                        planned.push((i, 0, Planned::Keep(i)));
                        planned.len() - 1
                    });
                    seq.push(slot);
                }
                Run::Bundle(members) if shared_runs.contains(&members) => {
                    let slot = *shared_slots.entry(members.clone()).or_insert_with(|| {
                        planned.push((
                            members[0],
                            0,
                            Planned::Bundle {
                                path: shared_names[&members].clone(),
                                flavors: None,
                                members: members.clone(),
                            },
                        ));
                        planned.len() - 1
                    });
                    seq.push(slot);
                }
                Run::Bundle(members) => {
                    n += 1;
                    planned.push((
                        members[0],
                        fi + 1,
                        Planned::Bundle {
                            path: name(Some(&flavor), n),
                            flavors: Some(vec![flavor.clone()]),
                            members,
                        },
                    ));
                    seq.push(planned.len() - 1);
                }
            }
        }
        flavor_runs.push((flavor, seq));
    }
    // One list: by the position of each entry's first file, which keeps
    // every flavor's order.
    let mut order: Vec<usize> = (0..planned.len()).collect();
    order.sort_by_key(|&k| (planned[k].0, planned[k].1));
    let mut new_index = vec![0; planned.len()];
    for (new, &old) in order.iter().enumerate() {
        new_index[old] = new;
    }
    let orders = orders.map(|_| {
        flavor_runs
            .iter()
            .map(|(f, seq)| (f.clone(), seq.iter().map(|&k| new_index[k]).collect()))
            .collect()
    });
    let mut slots: Vec<Option<Planned>> = planned.into_iter().map(|(_, _, p)| Some(p)).collect();
    let planned = order
        .iter()
        .map(|&k| slots[k].take().expect("once"))
        .collect();
    (planned, orders)
}

/// The plan for one TOC list (see [`plan`]).
fn plan_shared(
    name: &dyn Fn(Option<&str>, usize) -> String,
    entries: &[(bool, Option<Vec<String>>)],
    flavors: &[String],
) -> Vec<Planned> {
    let everywhere = |i: usize| {
        entries[i]
            .1
            .as_ref()
            .is_none_or(|l| flavors.iter().all(|f| l.contains(f)))
    };
    let mut planned: Vec<(usize, Planned)> = Vec::new();
    let mut shared_count = 0;
    let mut flavor_count: HashMap<String, usize> = HashMap::new();
    let mut shared: Vec<usize> = Vec::new();
    let mut specific: Vec<(String, Vec<usize>)> =
        flavors.iter().map(|f| (f.clone(), Vec::new())).collect();
    let mut flush_shared = |planned: &mut Vec<(usize, Planned)>, shared: &mut Vec<usize>| {
        if !shared.is_empty() {
            shared_count += 1;
            planned.push((
                shared[0],
                Planned::Bundle {
                    path: name(None, shared_count),
                    flavors: None,
                    members: std::mem::take(shared),
                },
            ));
        }
    };
    let flush_specific = |planned: &mut Vec<(usize, Planned)>,
                          specific: &mut Vec<(String, Vec<usize>)>,
                          counts: &mut HashMap<String, usize>| {
        for (f, members) in specific.iter_mut() {
            if members.is_empty() {
                continue;
            }
            let n = counts.entry(f.clone()).or_default();
            *n += 1;
            planned.push((
                members[0],
                Planned::Bundle {
                    path: name(Some(f), *n),
                    flavors: Some(vec![f.clone()]),
                    members: std::mem::take(members),
                },
            ));
        }
    };
    for (i, (bundlable, restriction)) in entries.iter().enumerate() {
        if !bundlable {
            flush_shared(&mut planned, &mut shared);
            flush_specific(&mut planned, &mut specific, &mut flavor_count);
            planned.push((i, Planned::Keep(i)));
        } else if everywhere(i) {
            flush_specific(&mut planned, &mut specific, &mut flavor_count);
            shared.push(i);
        } else {
            flush_shared(&mut planned, &mut shared);
            for (f, members) in specific.iter_mut() {
                if restriction.as_ref().is_some_and(|l| l.contains(f)) {
                    members.push(i);
                }
            }
        }
    }
    flush_shared(&mut planned, &mut shared);
    flush_specific(&mut planned, &mut specific, &mut flavor_count);
    // Stable: by the position of each entry's first file.
    planned.sort_by_key(|(first, _)| *first);
    planned.into_iter().map(|(_, p)| p).collect()
}

/// One file of a bundle.
pub struct Member {
    /// The module ID or path that load errors name.
    pub id: String,
    pub text: Vec<u8>,
    /// Its own source map (one that decodes); without one, its lines map
    /// to `source`.
    pub map: Option<Arc<EncodedMap>>,
    pub source: String,
}

/// Line breaks as Lua counts them (`\r\n` and `\n\r` count once).
fn lines(text: &[u8]) -> u32 {
    let mut n = 0;
    let mut i = 0;
    while i < text.len() {
        let c = text[i];
        if c == b'\n' || c == b'\r' {
            if let Some(&next) = text.get(i + 1)
                && (next == b'\n' || next == b'\r')
                && next != c
            {
                i += 1;
            }
            n += 1;
        }
        i += 1;
    }
    n
}

fn lua_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' | '\\' => {
                out.push('\\');
                out.push(c);
            }
            '\n' => out.push_str("\\n"),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// Whether `word` occurs as an identifier in `text`.
fn mentions(text: &[u8], word: &str) -> bool {
    let w = word.as_bytes();
    let is_word = |c: u8| c.is_ascii_alphanumeric() || c == b'_';
    text.windows(w.len()).enumerate().any(|(i, window)| {
        window == w
            && (i == 0 || !is_word(text[i - 1]))
            && text.get(i + w.len()).is_none_or(|&c| !is_word(c))
    })
}

/// Where a member's segments in the bundle's map come from.
enum Piece {
    /// Its own map's segments, `offset` lines down, with its sources and
    /// names at these indices of the bundle's.
    Own {
        offset: u32,
        map: Arc<EncodedMap>,
        sources: Vec<u32>,
        names: Vec<u32>,
    },
    /// One segment per line: line `l` of `source` at line `offset + l`.
    Lines {
        offset: u32,
        source: u32,
        count: u32,
    },
}

impl Piece {
    /// Calls `f` with the piece's segments in the bundle, in order.
    fn segments(&self, f: &mut impl FnMut(Segment)) {
        match self {
            Piece::Own {
                offset,
                map,
                sources,
                names,
            } => map.for_each_segment(|s| {
                f(Segment {
                    out_line: s.out_line + offset,
                    source: sources.get(s.source as usize).copied().unwrap_or(0),
                    name: s.name.and_then(|n| names.get(n as usize).copied()),
                    ..s
                })
            }),
            Piece::Lines {
                offset,
                source,
                count,
            } => {
                for l in 1..=*count {
                    f(Segment {
                        out_line: offset + l,
                        out_col: 0,
                        source: *source,
                        src_line: l,
                        src_col: 0,
                        name: None,
                    });
                }
            }
        }
    }
}

/// A list of unique strings with their indices.
#[derive(Default)]
struct Indexed {
    list: Vec<String>,
    index: HashMap<String, u32>,
}

impl Indexed {
    /// The index of `s`, added at the end when it is new.
    fn index_of(&mut self, s: &str) -> u32 {
        if let Some(&i) = self.index.get(s) {
            return i;
        }
        let i = self.list.len() as u32;
        self.list.push(s.to_string());
        self.index.insert(s.to_string(), i);
        i
    }
}

/// The text of a bundle, and its source map when `want_map`.
pub fn assemble(
    file: &str,
    build_id: &str,
    members: Vec<Member>,
    want_map: bool,
) -> (Vec<u8>, Option<EncodedMap>) {
    // The frame's names must not be names the files use (hygiene).
    let mut suffix = String::new();
    let mut n = 0;
    while ["__wl_name", "__wl_ns", "__wl_load"].iter().any(|base| {
        let name = format!("{base}{suffix}");
        members.iter().any(|m| mentions(&m.text, &name))
    }) {
        n += 1;
        suffix = n.to_string();
    }
    let (name, ns, load) = (
        format!("__wl_name{suffix}"),
        format!("__wl_ns{suffix}"),
        format!("__wl_load{suffix}"),
    );
    let mut text = format!(
        "local {name}, {ns} = ...\n\
         local function {load}(id, f)\n\
         xpcall(function() f({name}, {ns}) end, function(e)\n\
         local message = {name} .. \" (\" .. id .. \", build {build_id}): \" .. tostring(e)\n\
         local handler = geterrorhandler and geterrorhandler()\n\
         if handler then handler(message) else print(message) end\n\
         end)\n\
         end\n"
    )
    .into_bytes();
    let mut sources = Indexed::default();
    let mut names = Indexed::default();
    let mut pieces = Vec::new();
    let mut line = lines(&text) + 1;
    for member in members {
        text.extend_from_slice(
            format!("{load}({}, function(...)\n", lua_string(&member.id)).as_bytes(),
        );
        line += 1;
        let body = member
            .text
            .strip_prefix(b"\xEF\xBB\xBF")
            .unwrap_or(&member.text);
        let offset = line - 1;
        if want_map {
            pieces.push(match member.map {
                Some(own) => Piece::Own {
                    offset,
                    sources: own.sources.iter().map(|s| sources.index_of(s)).collect(),
                    names: own.names.iter().map(|s| names.index_of(s)).collect(),
                    map: own,
                },
                None => Piece::Lines {
                    offset,
                    source: sources.index_of(&member.source),
                    count: lines(body) + 1,
                },
            });
        }
        text.extend_from_slice(body);
        let mut body_lines = lines(body);
        if !body.ends_with(b"\n") && !body.ends_with(b"\r") {
            text.push(b'\n');
            body_lines += 1;
        }
        line += body_lines;
        text.extend_from_slice(b"end)\n");
        line += 1;
    }
    if !want_map {
        return (text, None);
    }
    // The pieces' segments are in output order unless a member's map is
    // longer than its text; then they are sorted, as they always were.
    let mut encoder = SegmentEncoder::default();
    let mut in_order = true;
    for piece in &pieces {
        piece.segments(&mut |s| in_order = encoder.push(&s));
        if !in_order {
            break;
        }
    }
    let map = match encoder.finish(file.to_string(), sources.list.clone(), names.list.clone()) {
        Some(map) => map,
        None => SourceMap {
            file: file.to_string(),
            sources: sources.list,
            names: names.list,
            segments: collect(&pieces),
        }
        .encode(),
    };
    // Debug builds (and so the tests) make the map the old way and compare.
    #[cfg(debug_assertions)]
    {
        let old = SourceMap {
            file: map.file.clone(),
            sources: map.sources.clone(),
            names: map.names.clone(),
            segments: collect(&pieces),
        };
        assert_eq!(map.to_json(), old.to_json(), "the bundle map of {file}");
    }
    (text, Some(map))
}

/// Every piece's segments, in piece order.
fn collect(pieces: &[Piece]) -> Vec<Segment> {
    let mut segments = Vec::new();
    for piece in pieces {
        piece.segments(&mut |s| segments.push(s));
    }
    segments
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `assemble` before streaming, with member maps as `SourceMap`s.
    fn old_assemble(
        file: &str,
        build_id: &str,
        members: Vec<(Member, Option<SourceMap>)>,
    ) -> (Vec<u8>, SourceMap) {
        fn index_of(list: &mut Vec<String>, s: &str) -> u32 {
            match list.iter().position(|x| x == s) {
                Some(i) => i as u32,
                None => {
                    list.push(s.to_string());
                    list.len() as u32 - 1
                }
            }
        }
        let mut suffix = String::new();
        let mut n = 0;
        while ["__wl_name", "__wl_ns", "__wl_load"].iter().any(|base| {
            let name = format!("{base}{suffix}");
            members.iter().any(|(m, _)| mentions(&m.text, &name))
        }) {
            n += 1;
            suffix = n.to_string();
        }
        let (name, ns, load) = (
            format!("__wl_name{suffix}"),
            format!("__wl_ns{suffix}"),
            format!("__wl_load{suffix}"),
        );
        let mut text = format!(
            "local {name}, {ns} = ...\n\
             local function {load}(id, f)\n\
             xpcall(function() f({name}, {ns}) end, function(e)\n\
             local message = {name} .. \" (\" .. id .. \", build {build_id}): \" .. tostring(e)\n\
             local handler = geterrorhandler and geterrorhandler()\n\
             if handler then handler(message) else print(message) end\n\
             end)\n\
             end\n"
        )
        .into_bytes();
        let mut map = SourceMap {
            file: file.to_string(),
            ..SourceMap::default()
        };
        let mut line = lines(&text) + 1;
        for (member, own) in members {
            text.extend_from_slice(
                format!("{load}({}, function(...)\n", lua_string(&member.id)).as_bytes(),
            );
            line += 1;
            let body = member
                .text
                .strip_prefix(b"\xEF\xBB\xBF")
                .unwrap_or(&member.text);
            let offset = line - 1;
            match own {
                Some(own) => {
                    let sources: Vec<u32> = own
                        .sources
                        .iter()
                        .map(|s| index_of(&mut map.sources, s))
                        .collect();
                    let names: Vec<u32> = own
                        .names
                        .iter()
                        .map(|s| index_of(&mut map.names, s))
                        .collect();
                    for s in own.segments {
                        map.segments.push(Segment {
                            out_line: s.out_line + offset,
                            source: sources.get(s.source as usize).copied().unwrap_or(0),
                            name: s.name.and_then(|n| names.get(n as usize).copied()),
                            ..s
                        });
                    }
                }
                None => {
                    let source = index_of(&mut map.sources, &member.source);
                    for l in 1..=lines(body) + 1 {
                        map.segments.push(Segment {
                            out_line: offset + l,
                            out_col: 0,
                            source,
                            src_line: l,
                            src_col: 0,
                            name: None,
                        });
                    }
                }
            }
            text.extend_from_slice(body);
            let mut body_lines = lines(body);
            if !body.ends_with(b"\n") && !body.ends_with(b"\r") {
                text.push(b'\n');
                body_lines += 1;
            }
            line += body_lines;
            text.extend_from_slice(b"end)\n");
            line += 1;
        }
        (text, map)
    }

    /// A small deterministic generator (xorshift), so failures reproduce.
    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
    }

    #[test]
    fn streamed_bundle_maps_match_the_old_ones() {
        const LINES: &[&[u8]] = &[b"local a = 1", b"print(a)", b"", b"-- c", b"x = y"];
        const BREAKS: &[&[u8]] = &[b"\n", b"\r", b"\r\n", b"\n\r"];
        const SOURCES: &[&str] = &["src/a.lua", "src/b.wowlua", "libs/x.lua", "src/\"q\".lua"];
        const NAMES: &[&str] = &["count", "value", "self", "x"];
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let mut fallbacks = 0;
        for round in 0..3000 {
            let mut new_members = Vec::new();
            let mut old_members = Vec::new();
            for m in 0..rng.below(5) {
                let mut text = Vec::new();
                if rng.below(5) == 0 {
                    text.extend_from_slice(b"\xEF\xBB\xBF");
                }
                let body_lines = rng.below(6);
                for l in 0..body_lines {
                    text.extend_from_slice(LINES[rng.below(LINES.len() as u64) as usize]);
                    if l + 1 < body_lines || rng.below(2) == 0 {
                        text.extend_from_slice(BREAKS[rng.below(4) as usize]);
                    }
                }
                let own = (rng.below(3) > 0).then(|| {
                    // Sometimes longer than the text: the fallback sorts.
                    let span = body_lines as u32 + 1 + rng.below(3) as u32 * rng.below(4) as u32;
                    let sources: Vec<String> = (0..1 + rng.below(2))
                        .map(|_| SOURCES[rng.below(SOURCES.len() as u64) as usize].to_string())
                        .collect();
                    let names: Vec<String> = (0..rng.below(3))
                        .map(|_| NAMES[rng.below(NAMES.len() as u64) as usize].to_string())
                        .collect();
                    let mut segments: Vec<Segment> = (0..rng.below(8))
                        .map(|_| Segment {
                            out_line: 1 + rng.below(u64::from(span)) as u32,
                            out_col: rng.below(30) as u32,
                            source: rng.below(sources.len() as u64 + 1) as u32,
                            src_line: 1 + rng.below(20) as u32,
                            src_col: rng.below(30) as u32,
                            name: (rng.below(2) == 0)
                                .then(|| rng.below(names.len() as u64 + 1) as u32),
                        })
                        .collect();
                    segments.sort_by_key(|s| (s.out_line, s.out_col));
                    SourceMap {
                        file: format!("m{m}.lua"),
                        sources,
                        names,
                        segments,
                    }
                });
                let member = |map| Member {
                    id: format!("src/m{m}"),
                    text: text.clone(),
                    map,
                    source: SOURCES[(m % 4) as usize].to_string(),
                };
                // A member map is what decoding the written one gives.
                let encoded = own.as_ref().map(|o| Arc::new(o.encode()));
                let decoded = encoded.as_ref().map(|e| e.decode().unwrap());
                new_members.push(member(encoded));
                old_members.push((member(None), decoded));
            }
            let (old_text, old_map) = old_assemble("A.lua", "b1", old_members);
            let sorted = old_map
                .segments
                .is_sorted_by_key(|s| (s.out_line, s.out_col));
            fallbacks += usize::from(!sorted);
            let without: Vec<Member> = new_members
                .iter()
                .map(|m| Member {
                    id: m.id.clone(),
                    text: m.text.clone(),
                    map: m.map.clone(),
                    source: m.source.clone(),
                })
                .collect();
            let (text, map) = assemble("A.lua", "b1", new_members, true);
            assert_eq!(text, old_text, "round {round}");
            assert_eq!(map.unwrap().to_json(), old_map.to_json(), "round {round}");
            let (text, map) = assemble("A.lua", "b1", without, false);
            assert!(text == old_text && map.is_none(), "round {round}");
        }
        assert!(fallbacks > 50, "{fallbacks} bundles took the fallback");
    }

    fn flavors(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn plans_segments_around_separate_files() {
        // runtime, lib, a, frames.xml, b
        let entries = vec![
            (true, None),
            (true, None),
            (true, None),
            (false, None),
            (true, None),
        ];
        let (planned, orders) = plan("A", &entries, &flavors(&["standard", "tbc"]), None);
        assert_eq!(
            planned,
            [
                Planned::Bundle {
                    path: "A.lua".into(),
                    flavors: None,
                    members: vec![0, 1, 2]
                },
                Planned::Keep(3),
                Planned::Bundle {
                    path: "A.2.lua".into(),
                    flavors: None,
                    members: vec![4]
                },
            ]
        );
        assert!(orders.is_none());
    }

    #[test]
    fn split_flavor_files_are_bundled_per_flavor() {
        // Every file has a variant per flavor (split mode).
        let entries = vec![
            (true, Some(flavors(&["standard"]))),
            (true, Some(flavors(&["tbc"]))),
            (true, Some(flavors(&["standard"]))),
            (true, Some(flavors(&["tbc"]))),
            (true, None),
        ];
        let (planned, _) = plan("A", &entries, &flavors(&["standard", "tbc"]), None);
        let members: Vec<(String, Vec<usize>)> = planned
            .iter()
            .filter_map(|p| match p {
                Planned::Bundle { path, members, .. } => Some((path.clone(), members.clone())),
                Planned::Keep(_) => None,
            })
            .collect();
        assert_eq!(
            members,
            [
                ("A.standard.lua".to_string(), vec![0, 2]),
                ("A.tbc.lua".to_string(), vec![1, 3]),
                ("A.lua".to_string(), vec![4]),
            ]
        );
    }

    #[test]
    fn flavors_with_different_files_get_bundles_of_their_own() {
        let entries = vec![
            (true, None),
            (true, Some(flavors(&["tbc"]))),
            (false, None),
            (true, None),
            (false, None),
            (true, Some(flavors(&["standard"]))),
        ];
        let (planned, _) = plan("A", &entries, &flavors(&["standard", "tbc"]), None);
        let paths: Vec<String> = planned
            .iter()
            .map(|p| match p {
                Planned::Keep(i) => format!("keep {i}"),
                Planned::Bundle { path, members, .. } => format!("{path} {members:?}"),
            })
            .collect();
        assert_eq!(
            paths,
            [
                // Files every flavor loads are shared; the others go into
                // bundles of their flavor.
                "A.lua [0]",
                "A.tbc.lua [1]",
                "keep 2",
                "A.2.lua [3]",
                "keep 4",
                "A.standard.lua [5]"
            ]
        );
    }

    #[test]
    fn assembles_protected_files_with_a_map() {
        let (text, map) = assemble(
            "A.lua",
            "abc123",
            vec![
                Member {
                    id: "libs/x".into(),
                    text: b"local a = 1\nprint(a)".to_vec(),
                    map: None,
                    source: "libs/x/x.lua".into(),
                },
                Member {
                    id: "src/main".into(),
                    text: b"error('boom')\n".to_vec(),
                    map: None,
                    source: "src/main.wowlua".into(),
                },
                Member {
                    id: "src/after".into(),
                    text: b"print('after', ...)\n".to_vec(),
                    map: None,
                    source: "src/after.wowlua".into(),
                },
            ],
            true,
        );
        let map = map.unwrap().decode().unwrap();
        let lua = mlua::Lua::new();
        lua.load(
            "__out = {} print = function(...) local t = {} for i = 1, select('#', ...) do t[i] = tostring((select(i, ...))) end __out[#__out + 1] = table.concat(t, ' ') end",
        )
        .exec()
        .unwrap();
        lua.load(&text[..])
            .call::<()>(("Addon", lua.create_table().unwrap()))
            .unwrap();
        let out: String = lua.load("return table.concat(__out, '|')").eval().unwrap();
        let out = out.replace("\\", "/");
        assert!(
            out.starts_with("1|Addon (src/main, build abc123): "),
            "{out}"
        );
        assert!(
            out.ends_with("boom|after Addon table: ") || out.contains("|after Addon"),
            "{out}"
        );
        // `print(a)` of libs/x is on its line 2.
        let text = String::from_utf8(text).unwrap();
        let line = text.lines().position(|l| l == "print(a)").unwrap() as u32 + 1;
        let seg = map.segments.iter().find(|s| s.out_line == line).unwrap();
        assert_eq!(
            (map.sources[seg.source as usize].as_str(), seg.src_line),
            ("libs/x/x.lua", 2)
        );
    }
}
