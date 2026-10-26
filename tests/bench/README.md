# Shaping bench

wowlua's benchmark suite for the optimizations that shape code for Lua 5.1
([Optimization §15.2.3](../../spec/toolchain/optimization.md#1523-shaping-for-lua-51),
roadmap M11). The addon's modules are written as addons write them, typed
in strict mode; `tests/shaping.test.wowlua` calls them in `@bench`
functions.

```console
$ cd tests/bench && wowlua bench
```

compares each benchmark in the build the `release` profile optimizes with
the same build without optimizations, on the embedded Lua 5.1.

Each optimization also has a profile of its own that turns on only it, to
measure it alone:

```console
$ wowlua bench --profile only-numeric-loops
```
