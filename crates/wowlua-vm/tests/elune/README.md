# elune's taint conformance cases

`luatest_scriptcases.lua` is copied unchanged from
[Meorawr/elune](https://github.com/Meorawr/elune) (commit
bcdb36d, `tests/luatest_scriptcases.lua`), under the MIT
license in `LICENSE`. Its cases pass in the WoW client when run securely;
`tests/taint.rs` runs them on wowlua's VM.
