# Typed Demo

A wowlua sample addon checked in **strict mode**: it tracks the gold earned
and spent in a session.

- `src/ring.wowlua`: a generic class (`Ring<T>`), a fixed-size buffer.
- `src/money.wowlua`: an enum, interfaces, a union type with a type
  predicate (`change is Earned`) and money formatting.
- `src/ledger.wowlua`: the ledger class (private fields, an accessor,
  `T?` results).
- `src/main.wowlua`: events and the `/tdemo` slash command. The build is
  shared between `standard` and `vanilla` (`build.flavor-mode = "shared"`),
  so the warband-bank API, which only the mainline client has, is used
  behind a flavor guard (`WOW_PROJECT_ID == WOW_PROJECT_MAINLINE`); without
  it the build fails with E2030.
- `src/saved.d.wowlua`: the type of the saved variable `TypedDemoDB`.
- `wowlua:log`: a session summary every 30 seconds (`C_Timer.NewTicker`) and
  each money change, shown in the dev window's Log tab (`/wl`).
- `/tdemo check` passes a value from the WoW API that can be `nil` to a
  `@checked` function: the runtime check of dev builds catches it.

Build it and copy it into your WoW clients:

```sh
wowlua check      # strict mode: no diagnostics
wowlua dev        # or: wowlua dev --link
```

In game, earn or spend some gold and type `/tdemo`.
