# Widget Demo

A wowlua sample addon: a movable window built from **widget classes**
(classes that extend native frame types, see spec §3.2).

- `src/ui/widgets.wowlua`: a counter button (and a subclass overriding it), a
  progress bar, a check button and an input box; `@script` methods handle
  their frame scripts.
- `src/ui/row.wowlua` and `src/ui/templates.xml`: a widget class exposed with
  `@global` as the mixin of an XML template.
- `src/ui/window.wowlua`: the window composing them.
- `src/main.wowlua`: the `/wdemo` slash command, with a user decorator.

Build it and copy it into your WoW clients:

```sh
wowlua build
wowlua dev        # or: wowlua dev --link
```

In game, type `/wdemo`.
