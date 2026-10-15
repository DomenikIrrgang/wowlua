//! Differential tests: every snippet runs in wowlua's VM and in PUC Lua 5.1
//! (through mlua); printed output and the final error must be identical.

use std::cell::RefCell;
use std::rc::Rc;

use wowlua_vm::{Value, Vm};

/// Output lines, then `error: <message>` if the chunk failed.
fn ours(src: &str) -> Vec<String> {
    let out = Rc::new(RefCell::new(Vec::<u8>::new()));
    let mut vm = Vm::new();
    let sink = out.clone();
    vm.set_output(move |b| sink.borrow_mut().extend_from_slice(b));
    let result = match vm.load(src.as_bytes(), "=test") {
        Ok(f) => vm.call(f, &[]).map(|_| ()).map_err(|e| match e.value {
            Value::Str(s) => String::from_utf8_lossy(vm.str_bytes(s)).into_owned(),
            other => format!("({})", other.type_name()),
        }),
        Err(msg) => Err(msg),
    };
    let mut lines: Vec<String> = String::from_utf8_lossy(&out.borrow())
        .lines()
        .map(str::to_string)
        .collect();
    if let Err(e) = result {
        lines.push(format!("error: {e}"));
    }
    normalize(lines)
}

fn puc(src: &str) -> Vec<String> {
    let lua = mlua::Lua::new();
    let out = Rc::new(RefCell::new(Vec::<String>::new()));
    let sink = out.clone();
    // print as Lua 5.1's: tostring of each argument, tab-separated.
    let print = lua
        .create_function(move |lua, args: mlua::MultiValue| {
            let tostring: mlua::Function = lua.globals().get("tostring")?;
            let mut parts = Vec::new();
            for a in args {
                let s: mlua::Value = tostring.call(a)?;
                match s {
                    mlua::Value::String(s) => {
                        parts.push(String::from_utf8_lossy(&s.as_bytes()).into_owned())
                    }
                    mlua::Value::Number(n) => parts.push(wowlua_vm::number::fmt_num(n)),
                    mlua::Value::Integer(n) => parts.push(n.to_string()),
                    _ => {
                        return Err(mlua::Error::runtime(
                            "'tostring' must return a string to 'print'",
                        ));
                    }
                }
            }
            for line in parts.join("\t").split('\n') {
                sink.borrow_mut().push(line.to_string());
            }
            Ok(())
        })
        .unwrap();
    lua.globals().set("print", print).unwrap();
    let result = lua.load(src).set_name("=test").exec();
    let mut lines = out.borrow().clone();
    if let Err(e) = result {
        let msg = match e {
            mlua::Error::RuntimeError(m) | mlua::Error::SyntaxError { message: m, .. } => m,
            mlua::Error::CallbackError { cause, .. } => cause.to_string(),
            other => other.to_string(),
        };
        // mlua appends a traceback to runtime errors.
        let msg = msg
            .split("\nstack traceback:")
            .next()
            .unwrap_or("")
            .to_string();
        lines.push(format!("error: {msg}"));
    }
    normalize(lines)
}

fn normalize(lines: Vec<String>) -> Vec<String> {
    regex_lite(&lines)
}

/// Replaces the addresses after `table: ` and the like so object
/// identities compare equal (`0x…` on Unix, bare hex digits on Windows).
fn regex_lite(lines: &[String]) -> Vec<String> {
    lines.iter().map(|l| without_addresses(l)).collect()
}

fn without_addresses(l: &str) -> String {
    let mut out = String::new();
    let mut rest = l;
    loop {
        let next = ["table: ", "function: ", "thread: ", "userdata: "]
            .iter()
            .filter_map(|p| rest.find(p).map(|i| (i, p.len())))
            .min();
        let Some((i, len)) = next else { break };
        out.push_str(&rest[..i + len]);
        let after = &rest[i + len..];
        let digits = after.strip_prefix("0x").unwrap_or(after);
        let n = digits.chars().take_while(|c| c.is_ascii_hexdigit()).count();
        rest = if n >= 6 {
            out.push_str("ADDR");
            &digits[n..]
        } else {
            after
        };
    }
    out.push_str(rest);
    out
}

#[track_caller]
fn same(src: &str) {
    let a = ours(src);
    let b = puc(src);
    assert_eq!(
        a,
        b,
        "\n--- source ---\n{src}\n--- ours ---\n{}\n--- PUC ---\n{}\n",
        a.join("\n"),
        b.join("\n")
    );
}

#[test]
fn arithmetic_and_numbers() {
    same("print(1 + 2, 7 - 10, 3 * 4, 7 / 2, 7 % 3, -7 % 3, 7 % -3, 2 ^ 10, -2 ^ 2)");
    same("print(1/3, 100, 1e15, 1e14, 123456789012345, 0.1 + 0.2, -0)");
    same("local a = 0 print(-a, 1/0, -1/0)");
    same("print('10' + 5, '3' * '4', '0x10' + 0, ' 2 ' * 2)");
    same("print(10 == '10', 2 < 3, 'a' < 'b', 'abc' < 'abd', 'Z' < 'a')");
}

#[test]
fn strings_and_concat() {
    same("print('a' .. 'b' .. 1 .. 2.5)");
    same("local s = 'x' for i = 1, 3 do s = s .. i end print(s)");
    same("print(#'hello', #'')");
    same("print(1 .. '')");
}

#[test]
fn locals_and_assignment() {
    same("local a, b, c = 1, 2 print(a, b, c)");
    same("local a, b = 1, 2, 3 print(a, b)");
    same("local a, b = 1, 2 a, b = b, a print(a, b)");
    same("local t = {} local i = 1 i, t[i] = i + 1, 20 print(i, t[1], t[2])");
    same("x, y = 1 print(x, y)");
    same("local t = {1, 2, 3} local i = 1 t[i], i = 10, 2 print(t[1], t[2], i)");
}

#[test]
fn control_flow() {
    same(
        "for i = 1, 3 do print(i) end for i = 3, 1, -1 do print(i) end for i = 1, 0 do print('no') end",
    );
    same("for i = 1, 2, 0.5 do print(i) end");
    same("local i = 0 while i < 3 do i = i + 1 if i == 2 then break end end print(i)");
    same("local i = 0 repeat local j = i i = i + 1 until j >= 2 print(i)");
    same("local x = 5 if x > 10 then print('a') elseif x > 3 then print('b') else print('c') end");
    same("for _, v in ipairs({10, 20, nil, 40}) do print(v) end");
    same("print(1 and 2, nil and 1, false or 'x', nil or false, 1 or error('no'))");
    same("local a = nil print(not a, not not a, a == nil, a ~= nil)");
    same(
        "local t = {} for i = 1, 3 do t[#t + 1] = function() return i end end print(t[1](), t[2](), t[3]())",
    );
    same("for k, v in next, {5} do print(k, v) end");
}

#[test]
fn functions_and_varargs() {
    same("local function f(...) return select('#', ...), ... end print(f(1, nil, 3))");
    same(
        "local function f(a, b, ...) local t = {...} return a, b, #t end print(f(1)) print(f(1, 2, 3, 4))",
    );
    same(
        "local function f() return 1, 2, 3 end print(f()) print((f())) print(f(), 10) local t = {f(), f()} print(#t)",
    );
    same("print(select(2, 'a', 'b', 'c')) print(select(-1, 'a', 'b', 'c'))");
    same("print(unpack({1, 2, 3})) print(unpack({1, 2, 3}, 2)) print(unpack({1, 2, 3}, 2, 5))");
    same(
        "local function fact(n) if n <= 1 then return 1 end return n * fact(n - 1) end print(fact(10))",
    );
    same(
        "local function count(n) if n == 0 then return 'done' end return count(n - 1) end print(count(50000))",
    );
    same(
        "local function mk() local c = 0 return function() c = c + 1 return c end end local f, g = mk(), mk() print(f(), f(), g())",
    );
    same(
        "local t = {} function t.f(x) return x * 2 end function t:g(x) return self.k + x end t.k = 1 print(t.f(3), t:g(2))",
    );
}

#[test]
fn tables() {
    same("local t = {1, 2, 3, x = 1, [10] = 5} print(#t, t.x, t[10])");
    same("local t = {} t[1] = 1 t[2] = 2 t[4] = 4 print(#t)");
    same(
        "local t = {1, 2, nil, 4} print(#t) local u = {nil, nil, 3} print(#u) local v = {} v[5] = 1 print(#v)",
    );
    same("local t = {} for i = 1, 10 do t[i] = i end for i = 1, 10, 2 do t[i] = nil end print(#t)");
    same(
        "local t = {} for _, k in ipairs({'b', 'a', 'zz', 'name', 'x1', 'x2', 'text', 'value'}) do t[k] = true end local o = {} for k in pairs(t) do o[#o + 1] = k end print(unpack(o))",
    );
    same(
        "local t = {10, 20, 30, a = 1, b = 2, [1.5] = 3, [true] = 4} local o = {} for k, v in pairs(t) do o[#o + 1] = tostring(k) .. '=' .. tostring(v) end print(unpack(o))",
    );
    same("local t = {n = 0} t.n = t.n + 1 print(t.n, t['n'])");
    same("local t = {[1.0] = 'one', [2] = 'two'} print(t[1], t[2.0])");
    same("local t = {} for i = 1, 100 do t[i] = i end print(#t, t[50])");
    same(
        "local t = setmetatable({}, {__index = function(t, k) return k .. '!' end}) print(t.hi, rawget(t, 'hi'))",
    );
    same(
        "local log = {} local t = setmetatable({}, {__newindex = function(t, k, v) rawset(t, k, v * 2) end}) t.a = 2 print(t.a)",
    );
    same(
        "local mt = {__add = function(a, b) return 'added' end, __concat = function(a, b) return 'cat' end, __unm = function() return 'neg' end, __call = function(self, x) return 'called', x end} local t = setmetatable({}, mt) print(t + 1, 1 + t, t .. 'x', 'x' .. t, -t, t(5))",
    );
    same(
        "local mt = {__eq = function() return true end, __lt = function() return true end, __le = function() return false end} local a, b = setmetatable({}, mt), setmetatable({}, mt) print(a == b, a < b, a <= b, a ~= b)",
    );
    same(
        "local t = setmetatable({}, {__tostring = function() return 'T!' end}) print(t, tostring(t))",
    );
    same("print(getmetatable(setmetatable({}, {__metatable = 'locked'})))");
    same("print(pcall(setmetatable, setmetatable({}, {__metatable = 1}), {}))");
}

#[test]
fn errors_name_their_operands() {
    same("local x print(x.y)");
    same("print(undefined_global.field)");
    same("local t = {} print(t.a.b)");
    same("local t = {} t.x.y = 1");
    same("local s = 'x' .. {}");
    same("local n = 1 + {}");
    same("local a = {} local b = a + 1");
    same("undefined_function()");
    same("local t = {} t.method()");
    same("local t = {} t:method()");
    same("local f = 1 f()");
    same("print(1 < 'x')");
    same("print({} < {})");
    same("print(#nil)");
    same("local up = nil local function f() return up.x end f()");
    same("for i = 'a', 2 do end");
    same("local t = {} t[nil] = 1");
    same("aaa = {} x = (aaa or aaa) + (aaa and aaa)");
    same("aaa = {} (aaa or aaa)()");
}

#[test]
fn error_handling() {
    same("print(pcall(error, 'msg'))");
    same("print(pcall(error, 'msg', 0))");
    same("print(pcall(function() error('deep') end))");
    same("print(pcall(function() error('lvl2', 2) end))");
    same("print(pcall(function() error({}) end))");
    same("print(pcall(function() error() end))");
    same("print(pcall(function() local x = nil; return x.y end))");
    same("print(xpcall(function() error('boom') end, function(m) return 'handled: ' .. m end))");
    same("print(pcall(assert, false)) print(pcall(assert, nil, 'why')) print(assert(1, 2, 3))");
    same("print(select('#', pcall(error)))");
    same("error('top level')");
    same("error('no position', 0)");
    same(
        "local ok, e = pcall(function() local t = setmetatable({}, {__index = function() error('in index') end}) return t.x end) print(ok, e)",
    );
}

#[test]
fn native_environments() {
    same("local f = coroutine.wrap(function() end) print(setfenv(f, {}) == f, getfenv(f) == _G)");
    same("print(getfenv(print) == _G, getfenv(coroutine.wrap(print)) == _G)");
    same(
        "local e = {} local f = coroutine.wrap(function() end) setfenv(f, e) print(getfenv(f) == e)",
    );
    same("print(pcall(setfenv, print, {}))");
}

#[test]
fn syntax_errors() {
    same("print(loadstring('break x'))");
    same("print(loadstring('x = = 1'))");
    same("print(loadstring('x = \\200'))");
    same("print(loadstring('x = 1\\ry = = 2'))");
    same("print(loadstring('f(', 'name'))");
    same("print(loadstring('local t = {1,\\n2'))");
    same("print(loadstring(string.rep('a', 100) .. ' = = 1'))");
    same("print(loadstring('x = 1', '@' .. string.rep('d/', 40)))");
    same("print(loadstring('x = = 1', '=' .. string.rep('n', 80)))");
    // Limits met before the missing `end`s at the end of the file.
    same(
        "local s = 'function foo ()\\n  local ' for j = 1, 70 do s = s .. 'a' .. j .. ', ' end s = s .. 'b\\n' for j = 1, 70 do s = s .. 'function foo' .. j .. ' ()\\n a' .. j .. '=3\\n' end print(loadstring(s))",
    );
    same(
        "local s = '\\nfunction foo ()\\n  local ' for j = 1, 300 do s = s .. 'a' .. j .. ', ' end s = s .. 'b\\n' print(loadstring(s))",
    );
    // load stops calling the reader at a syntax error.
    same(
        "local i = 0 local s = '*a = 123' print(load(function () i = i + 1 return s:sub(i, i) end), i)",
    );
    // The first line: mlua's handler appends a traceback.
    same(
        "local f, e = load(function () error('in reader') end) print(f, (string.match(e, '^[^\\n]*')))",
    );
}

#[test]
fn base_library() {
    same("print(type(nil), type(1), type('s'), type({}), type(print), type(true))");
    same(
        "print(tonumber('10'), tonumber('0x1F'), tonumber('1e2'), tonumber('z'), tonumber('10', 2), tonumber('ff', 16), tonumber('zz', 36), tonumber(' 12 '))",
    );
    same("print(tostring(nil), tostring(true), tostring(12), tostring(1.5))");
    same("print(rawequal({}, {}), rawequal(1, 1), rawequal('a', 'a'))");
    same("local t = {} rawset(t, 'a', 1) print(rawget(t, 'a'))");
    same("print(next({}), type(next), select('#', next({})))");
    same("local f = loadstring('return 1 + ...') print(f(41)) print(loadstring('x ='))");
    same("print(pcall(tostring))");
    same("print(pcall(tonumber, 'x', 99))");
    same(
        "print(getfenv(1) == _G, getfenv(0) == _G) local env = {print = print} local function f() x = 1 end setfenv(f, env) f() print(env.x, x)",
    );
    same("print(_VERSION)");
}

#[test]
fn string_library() {
    same("print(string.len('abc'), ('abc'):len(), #'abc')");
    same(
        "print(('hello'):sub(2), ('hello'):sub(2, 3), ('hello'):sub(-3), ('hello'):sub(-3, -2), ('hello'):sub(0), ('hello'):sub(10), ('hello'):sub(3, 1))",
    );
    same(
        "print(('Hello World'):upper(), ('Hello World'):lower(), ('abc'):rep(3), ('abc'):rep(0), ('abc'):reverse())",
    );
    same("print(('ABC'):byte(), ('ABC'):byte(2), ('ABC'):byte(1, -1), ('ABC'):byte(10))");
    same("print(string.char(72, 105), string.char())");
    same("print(pcall(string.char, 256)) print(pcall(string.rep)) print(pcall(('x').rep, 'x'))");
    same("print(string.sub('hello', 2.7, 4.2))");
}

#[test]
fn string_format() {
    same(r#"print(string.format('%d %5d %-5d| %05d %+d % d %.3d', 42, 42, 42, 42, 42, 42, 7))"#);
    same(
        r#"print(string.format('%x %X %#x %o %#o %u %c%c', 255, 255, 255, 8, 8, 3000000000, 72, 105))"#,
    );
    same(
        r#"print(string.format('%f %.2f %10.3f %-10.1f| %e %.2E %g %g %g %G', 3.14159, 2.5, 1/3, 2.25, 12345.678, 0.00012, 100000, 1e20, 0.0001, 1e-10))"#,
    );
    same(r#"print(string.format('%g %g %g %g %.3g %#g', 1, 1.5, 123456789, 0.1, 2/3, 1))"#);
    same(r#"print(string.format('%s|%10s|%-10s|%.2s', 'abc', 'abc', 'abc', 'abc'))"#);
    same(r#"print(string.format('%q', 'a "quoted"\n line\\ \r end'))"#);
    same(
        r#"print(string.format('%5.1f%%', 99.44), string.format('%d', -0), string.format('%d', 3.9), string.format('%d', -3.9))"#,
    );
    same(
        r#"print(string.format('%s %s %s', 1, 1.5, 'x'), string.format('[%c]', 0), #string.format('%c', 0))"#,
    );
    same(
        r#"print(pcall(string.format, '%d', 'x')) print(pcall(string.format, '%y', 1)) print(pcall(string.format, '%d')) print(pcall(string.format, '%123d', 1)) print(pcall(string.format, '%------d', 1))"#,
    );
    same(
        r#"print(string.format('%5s|', 'ab'), string.format('%s', ('x'):rep(150)) == ('x'):rep(150))"#,
    );
    same(
        r#"print(string.format('%.14g', 0.1), string.format('%.20f', 0.1), string.format('%e', 0), string.format('%g', -0.0))"#,
    );
    // `%x` of a negative number is undefined in C: ARM saturates to 0,
    // x86-64 wraps (and Windows' `long` has 32 bits).
    same(
        r#"print(string.format('%x', 255), string.format('%5.2s|', 'abc'), string.format('%-+6d|', 5))"#,
    );
}

#[test]
fn string_patterns() {
    same(
        "print(('hello world'):find('o'), ('hello world'):find('o', 6), ('hello world'):find('xyz'), ('hello'):find(''), ('hello'):find('', 10))",
    );
    same("print(('a.b'):find('.', 1, true), ('a.b'):find('%.'), ('a+b'):find('+', 1, true))");
    same(
        "print(('hello'):find('l+'), ('hello'):match('(h)(e)'), ('hello'):match('()ll()'), ('key=value'):match('(%w+)=(%w+)'))",
    );
    same(
        "print(('  trim  '):match('^%s*(.-)%s*$'), ('abc'):match('^b'), ('abc'):match('c$'), ('a$c'):match('$c'))",
    );
    same(
        "print(('THE (quick) fox'):find('%((%a+)%)'), ('f(a(b)c)d'):match('%b()'), ('THE END'):find('%f[%a]%a+'))",
    );
    same(
        "print(('hello'):gsub('l', 'L'), ('hello'):gsub('l', 'L', 1), ('abc'):gsub('', '-'), ('hello world'):gsub('(%w+)', '<%1>'))",
    );
    same(
        "print(('hello'):gsub('.', {h = 'H', e = false}), ('x=1 y=2'):gsub('(%w+)=(%w+)', '%2=%1'), ('abc'):gsub('%w', '%0%0'))",
    );
    same(
        "print(('hello world'):gsub('%w+', function(w) return w:upper() end), ('abc'):gsub('b', function() return nil end))",
    );
    same(
        "local t = {} for w in ('one two  three'):gmatch('%a+') do t[#t + 1] = w end print(unpack(t))",
    );
    same("for k, v in ('a=1, b=2'):gmatch('(%w+)=(%w+)') do print(k, v) end");
    same("for p in ('abc'):gmatch('()') do print(p) end");
    same(
        "print(('aaa'):match('a-'), ('aaa'):match('a-$'), ('aaab'):match('a*b'), ('b'):match('a?b'), ('[x]'):match('[%[]'), ('x-y'):match('[%w-]+'))",
    );
    same(
        "print(('\\0a\\0'):find('%z'), ('abc123'):match('%d+'), ('ABCdef'):match('%u+'), ('a,b;c'):gsub('%p', ''))",
    );
    same(
        "print(pcall(string.find, 'a', '(')) print(pcall(string.find, 'a', '%')) print(pcall(string.find, 'a', '[a')) print(pcall(string.find, 'a', '%1'))",
    );
    same(
        "print(pcall(string.gsub, 'abc', '(a)', '%2')) print(pcall(string.gsub, 'abc', 'a', true)) print(pcall(string.gsub, 'abc', 'a', function() return {} end))",
    );
    same(
        "print(('hello'):gsub('l', '%%'), ('hello'):gsub('', 'x', 2), ('abc'):gsub('^a', 'A'), ('aaa'):gsub('^a', 'A'))",
    );
    same("print(('x'):rep(3, ','), ('abc'):find('b', -1), ('abc'):find('b', -2))");
}

#[test]
fn table_library() {
    same(
        "local t = {1, 2, 3} table.insert(t, 4) table.insert(t, 1, 0) print(table.concat(t, ','))",
    );
    same(
        "local t = {1, 2, 3} print(table.remove(t), table.remove(t, 1), table.concat(t, ',')) print(table.remove({}))",
    );
    same(
        "print(table.concat({1, 2, 'x', 4.5}, '-'), table.concat({}, 'x'), table.concat({1, 2, 3}, ', ', 2, 3))",
    );
    same(
        "print(pcall(table.concat, {1, {}, 3})) print(pcall(table.insert, {}, 1, 2, 3)) print(pcall(table.insert))",
    );
    same("print(table.getn({1, 2, 3}), table.maxn({1, 2, [10] = 3, [2.5] = 4}))");
    same(
        "table.foreachi({'a', 'b'}, print) print(table.foreach({x = 1}, function(k, v) return k .. v end))",
    );
    same("local t = {5, 2, 8, 1, 9, 3, 7, 4, 6, 0} table.sort(t) print(table.concat(t, ' '))");
    same(
        "local t = {5, 2, 8, 1, 9, 3, 7, 4, 6, 0} table.sort(t, function(a, b) return a > b end) print(table.concat(t, ' '))",
    );
    same(
        "local t = {'pear', 'apple', 'fig', 'banana', 'kiwi'} table.sort(t) print(table.concat(t, ' '))",
    );
    same(
        "local calls = 0 local t = {} for i = 1, 50 do t[i] = (i * 37) % 23 end table.sort(t, function(a, b) calls = calls + 1 return a < b end) print(calls, table.concat(t, ','))",
    );
    same(
        "local t = {{k = 2, n = 'a'}, {k = 1, n = 'b'}, {k = 2, n = 'c'}, {k = 1, n = 'd'}, {k = 2, n = 'e'}} table.sort(t, function(a, b) return a.k < b.k end) local o = {} for i, v in ipairs(t) do o[i] = v.n end print(table.concat(o))",
    );
    same("print(pcall(table.sort, {3, 1, 2}, function(a, b) return true end))");
    same("print(pcall(table.sort, {1, 'x', 2}))");
    same("print(pcall(table.setn, {}, 1))");
}

#[test]
fn math_library() {
    same(
        "print(math.floor(3.7), math.ceil(3.2), math.floor(-3.5), math.abs(-2), math.max(1, 5, 3), math.min(4, -1, 2))",
    );
    same(
        "print(math.sqrt(16), math.pi, math.huge, -math.huge, math.fmod(7, 3), math.fmod(-7, 3), math.modf(3.7), math.modf(-3.7))",
    );
    same(
        "print(math.deg(math.pi), math.rad(180), math.exp(0), math.log(1), math.log10(1000), math.pow(2, 10))",
    );
    same(
        "print(math.frexp(8), math.frexp(0.75), math.frexp(-3), math.ldexp(0.5, 4), math.frexp(0))",
    );
    same(
        "print(math.sin(0), math.cos(0), math.atan2(1, 1), math.floor(math.tan(math.pi / 4) * 1e10 + 0.5))",
    );
    same("print(pcall(math.floor, 'x')) print(pcall(math.max)) print(math.floor('3.9'))");
    same(
        "print(pcall(math.random, 0)) print(pcall(math.random, 3, 1)) print(pcall(math.random, 1, 2, 3))",
    );
    same(
        "for i = 1, 20 do local r = math.random(5) assert(r >= 1 and r <= 5 and r == math.floor(r)) end local f = math.random() assert(f >= 0 and f < 1) print('ok')",
    );
}

#[test]
fn coroutines() {
    same(
        "local co = coroutine.create(function(a, b) print('start', a, b) local c = coroutine.yield(a + b) print('got', c) local d, e = coroutine.yield(c * 2) return d + e end) print(coroutine.resume(co, 1, 2)) print(coroutine.resume(co, 10)) print(coroutine.resume(co, 3, 4)) print(coroutine.resume(co)) print(coroutine.status(co))",
    );
    same(
        "local gen = coroutine.wrap(function() for i = 1, 3 do coroutine.yield(i) end end) print(gen(), gen(), gen())",
    );
    same(
        "local co = coroutine.create(function() error('inside') end) print(coroutine.resume(co)) print(coroutine.status(co))",
    );
    same(
        "local co = coroutine.create(function() error({}) end) local ok, e = coroutine.resume(co) print(ok, type(e))",
    );
    same(
        "local co co = coroutine.create(function() print(coroutine.status(co), coroutine.running() == co) end) print(coroutine.status(co)) coroutine.resume(co) print(coroutine.status(co), coroutine.running())",
    );
    same("print(pcall(coroutine.yield, 1))");
    same(
        "local co = coroutine.create(function() pcall(coroutine.yield, 1) end) print(coroutine.resume(co))",
    );
    same(
        "local co = coroutine.create(function() local t = setmetatable({}, {__index = function() return coroutine.yield('mm') end}) return t.x end) print(coroutine.resume(co))",
    );
    same(
        "local co = coroutine.create(function() for k in function() return coroutine.yield('it') end do end end) print(coroutine.resume(co))",
    );
    same("print(pcall(coroutine.create, print)) print(pcall(coroutine.resume, 1))");
    same("local f = coroutine.wrap(function() error('wrapped') end) print(pcall(f))");
    same("local f = coroutine.wrap(function() return 1 end) f() print(pcall(f))");
    same(
        "local co = coroutine.create(function(...) return select('#', ...), ... end) print(coroutine.resume(co, nil, nil))",
    );
    same(
        "local function tail() return coroutine.yield('t') end local co = coroutine.create(function() local v = tail() return 'after ' .. v end) print(coroutine.resume(co)) print(coroutine.resume(co, 'x'))",
    );
    same(
        "local inner = coroutine.create(function() coroutine.yield('in1') return 'in-done' end) local outer = coroutine.create(function() print(coroutine.resume(inner)) coroutine.yield('out1') print(coroutine.resume(inner)) print(coroutine.status(inner)) end) print(coroutine.resume(outer)) print(coroutine.resume(outer))",
    );
    same(
        "local co = coroutine.create(function() local x = 0 local function inc() x = x + 1 return x end coroutine.yield(inc) return x end) local _, inc = coroutine.resume(co) inc() inc() print(coroutine.resume(co))",
    );
}

#[test]
fn garbage_collection_api() {
    same(
        "print(type(collectgarbage('count')), collectgarbage(), type(gcinfo())) print(pcall(collectgarbage, 'bogus'))",
    );
}
