//! The compiler's bytecode against Lua 5.1's own (`string.dump` of PUC Lua
//! with LUA_COMPAT_VARARG, hence the compat option), on snippets.

use wowlua_vm::compiler::{Options, compile_with};
use wowlua_vm::heap::Heap;
use wowlua_vm::luac;

fn listing(heap: &Heap, p: &wowlua_vm::proto::Proto) -> Vec<String> {
    let _ = heap;
    p.code
        .iter()
        .map(|op| format!("{:?}", luac::encode_ours(op)))
        .collect()
}

#[track_caller]
fn same(src: &str) {
    let lua = mlua::Lua::new();
    let f = lua.load(src).into_function().expect("Lua compiles it");
    let dump = f.dump(false);
    let theirs = luac::decode(&dump).expect("decodes");
    let mut heap = Heap::default();
    let mut db = wowlua_span::SourceDb::new();
    let file = db.add("t", src.to_string());
    let parsed = wowlua_syntax::parse(src.as_bytes(), file);
    let ours = compile_with(
        &mut heap,
        parsed.view(),
        src.as_bytes(),
        b"=t"[..].into(),
        Options {
            compat_vararg: true,
            ..Default::default()
        },
    )
    .expect("compiles");
    if let Some(d) = luac::compare(&heap, &ours, &theirs, "main") {
        let lua_list: Vec<String> = theirs
            .code
            .iter()
            .map(|i| format!("{:?}", luac_decode(*i)))
            .collect();
        panic!(
            "{src}\n{d}\nours:\n{}\nLua:\n{}",
            listing(&heap, &ours).join("\n"),
            lua_list.join("\n")
        );
    }
}

fn luac_decode(i: u32) -> (u32, u32, u32, u32) {
    (
        i & 0x3f,
        (i >> 6) & 0xff,
        (i >> 23) & 0x1ff,
        (i >> 14) & 0x1ff,
    )
}

/// Line numbers per instruction must match too (error messages use them).
#[track_caller]
fn same_lines(src: &str) {
    same(src);
    let lua = mlua::Lua::new();
    let dump = lua.load(src).into_function().expect("compiles").dump(false);
    let theirs = luac::decode(&dump).expect("decodes");
    let mut heap = Heap::default();
    let mut db = wowlua_span::SourceDb::new();
    let file = db.add("t", src.to_string());
    let parsed = wowlua_syntax::parse(src.as_bytes(), file);
    let ours = compile_with(
        &mut heap,
        parsed.view(),
        src.as_bytes(),
        b"=t"[..].into(),
        Options {
            compat_vararg: true,
            ..Default::default()
        },
    )
    .expect("compiles");
    fn walk(p: &wowlua_vm::proto::Proto, t: &luac::DumpProto, path: &str, out: &mut Vec<String>) {
        let mut j = 0;
        for (i, op) in p.code.iter().enumerate() {
            let theirs = t.lines.get(j).copied().unwrap_or(0);
            if p.lines[i] != theirs {
                out.push(format!(
                    "{path} #{i} {:?}: ours {} Lua {}",
                    luac::encode_ours(op),
                    p.lines[i],
                    theirs
                ));
            }
            j += 1;
            if let wowlua_vm::proto::Op::Closure { proto, .. } = op {
                j += p.protos[*proto as usize].upvals.len();
            }
        }
        for (k, (a, b)) in p.protos.iter().zip(&t.protos).enumerate() {
            walk(a, b, &format!("{path}/{k}"), out);
        }
    }
    let mut out = Vec::new();
    walk(&ours, &theirs, "main", &mut out);
    assert!(out.is_empty(), "{src}\n{}", out.join("\n"));
}

#[test]
fn lines() {
    same_lines("local a = 1\nlocal b = a +\n  2\nprint(a,\n b)\n");
    same_lines(
        "local t = {\n  1,\n  x = 2,\n}\nlocal function f(x)\n  return x\nend\nif t then\n  f(1)\nelseif t.x then\n  f(2)\nelse\n  f(3)\nend\n",
    );
    same_lines(
        "for i = 1, 3 do\n  print(i)\nend\nfor k, v in pairs({}) do\n  print(k)\nend\nwhile false do\n  break\nend\nrepeat\n  local x = 1\nuntil x\n",
    );
    same_lines(
        "local t = {}\nfunction t.f(a, b)\n  return a\n    + b\nend\nfunction t:m()\n  return self\nend\nt.x = t.f(\n  1,\n  2\n)\n",
    );
    same_lines("local x = nil\nlocal y\nx = x or\n  y and\n  1\nlocal s = 'a' ..\n  'b' ..\n  x\n");
}

#[test]
fn boolean_patterns() {
    same("local x return x and true or false");
    same("local a, b return a == b and 1 or 2");
    same("local a local b = not a");
    same("local a, b, c local d = a and b or c");
}

/// Lists line mismatches for one file (`WOWLUA_LUAC_FILE=path cargo test
/// -p wowlua-vm --test luac -- --ignored file_lines --nocapture`).
#[test]
#[ignore]
fn file_lines() {
    let path = std::env::var("WOWLUA_LUAC_FILE").expect("WOWLUA_LUAC_FILE");
    let src = std::fs::read_to_string(&path).expect("readable");
    let src = src.strip_prefix('\u{feff}').unwrap_or(&src).to_string();
    let lua = mlua::Lua::new();
    let dump = lua
        .load(&src)
        .into_function()
        .expect("compiles")
        .dump(false);
    let theirs = luac::decode(&dump).expect("decodes");
    let mut heap = Heap::default();
    let mut db = wowlua_span::SourceDb::new();
    let file = db.add("t", src.clone());
    let parsed = wowlua_syntax::parse(src.as_bytes(), file);
    let ours = compile_with(
        &mut heap,
        parsed.view(),
        src.as_bytes(),
        b"=t"[..].into(),
        Options {
            compat_vararg: true,
            ..Default::default()
        },
    )
    .expect("compiles");
    let lines: Vec<&str> = src.lines().collect();
    fn walk(p: &wowlua_vm::proto::Proto, t: &luac::DumpProto, lines: &[&str], n: &mut usize) {
        let mut j = 0;
        for (i, op) in p.code.iter().enumerate() {
            let theirs = t.lines.get(j).copied().unwrap_or(0);
            if p.lines[i] != theirs && *n < 25 {
                *n += 1;
                let show = |l: u32| lines.get(l as usize - 1).map_or("", |s| s.trim());
                println!(
                    "{:?}: ours {} `{}` | Lua {} `{}`",
                    luac::encode_ours(op),
                    p.lines[i],
                    show(p.lines[i]),
                    theirs,
                    show(theirs)
                );
            }
            j += 1;
            if let wowlua_vm::proto::Op::Closure { proto, .. } = op {
                j += p.protos[*proto as usize].upvals.len();
            }
        }
        for (a, b) in p.protos.iter().zip(&t.protos) {
            walk(a, b, lines, n);
        }
    }
    let mut n = 0;
    walk(&ours, &theirs, &lines, &mut n);
}
