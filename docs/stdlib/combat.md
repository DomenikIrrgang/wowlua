# Combat, secure code and hooks

Avoid "Interface action failed because of an AddOn": queue work until combat
ends, let the game show and hide frames during a fight, and run code after
Blizzard's functions without tainting them.

In combat the game locks protected frames. Until combat ends, an addon may
not show, hide or move them, or change a secure button's attributes. If it
tries, the player sees "Interface action failed because of an AddOn". These
three modules help you work within that lockdown.

## Wait for combat to end

Put `@outOfCombat` on a function that touches protected frames. Out of
combat it runs at once. In combat the call is queued, and runs as soon as
the fight ends (`PLAYER_REGEN_ENABLED`). If it is called ten times during a
fight, it runs once, with the last arguments.

```wowlua title=src/bars.wowlua
import outOfCombat from "wowlua:combat"

class ActionBar
    frame: Frame
    offset: number = 0

    constructor(frame: Frame)
        self.frame = frame
    end

    @outOfCombat
    function relayout()
        self.frame:ClearAllPoints()
        self.frame:SetPoint("BOTTOM", UIParent, "BOTTOM", 0, self.offset)
    end
end

export const bar = ActionBar.new(CreateFrame("Frame", nil, UIParent, "SecureHandlerStateTemplate"))

-- the settings slider can call this at any time, in combat or not
export function onOffsetChanged(value: number)
    bar.offset = value
    bar:relayout()
end
```

For a method, calls are collapsed per object. The function must not return
anything (E8030), because a queued call has no result to give back.

For a one-off, `defer` does the same with a function you pass. Give it a key,
and only the latest function queued with that key runs:

```wowlua title=src/secure-button.wowlua
import defer, inCombat, onCombatChange from "wowlua:combat"
import print from "wowlua:log"

local castButton = CreateFrame("Button", nil, UIParent, "SecureActionButtonTemplate")
local statusIcon = UIParent:CreateTexture(nil, "OVERLAY")

-- typing in a box during combat: only the final name is set
export function setSpell(spellName: string)
    defer(function()
        castButton:SetAttribute("spell", spellName)
    end, "castButton:spell")
    if inCombat() then
        print("Changes apply after combat.")
    end
end

onCombatChange(function(entering: boolean)
    statusIcon:SetShown(entering)
end)
```

Queued calls run in the order they were first queued. If combat starts
again while the queue runs, the rest waits for the next end of combat. A
queued function that raises an error is reported, and the others still run.

> [!NOTE]
> `inCombat()` is the lockdown (`InCombatLockdown`), which decides whether
> protected calls work. Whether the player is actually fighting is a
> different question, which this module doesn't answer.

| Function | What it does |
|----------|--------------|
| `@outOfCombat` | Runs now, or after combat; repeated calls collapse into one |
| `defer(fn, key?)` | The same for a function you pass |
| `inCombat()` | Whether protected calls are blocked right now |
| `onCombatChange(fn)` | Calls `fn` with true when combat starts and false when it ends |

## Show and hide frames in combat

Your code can't show a protected frame in combat, but the game can, if you
give it the rule up front. A state driver is a macro conditional that the
game evaluates itself, also in the middle of a fight.

```wowlua title=src/visibility.wowlua
import * as secure from "wowlua:secure"
import bar from "./bars"

local pageLabel = bar.frame:CreateFontString(nil, "OVERLAY", "GameFontNormal")

-- hidden in pet battles and vehicles, shown in combat, hidden otherwise
secure.visibility(bar.frame, "[petbattle][vehicleui] hide; [combat] show; hide")

-- a page that follows the bonus bar and Shift, with a label that follows it
secure.state(bar.frame, "page", "[bonusbar:5] 11; [mod:shift] 2; 1", function(page: string)
    pageLabel:SetText(page)
end)

if secure.evaluate("[group:raid] yes") then
    print("in a raid")
end
```

A literal conditional is checked when you build. An unpaired bracket or a
condition the client doesn't know, such as `[combta]`, is a build error
(E8440), not a bar that silently never shows. A `visibility` value other
than `show` or `hide` is E8441.

- Registering or removing a driver in combat waits for the end of combat by
  itself, and the drivers keep working through every fight.
- `state` calls your function with each new value, the first one right away.
  It runs as ordinary addon code, so in combat it may only change frames
  that aren't protected. Cancelling the subscription it returns removes the
  driver.
- `secure.visibility(frame, nil)` removes the driver and leaves the frame as
  it is.
- `evaluate` returns what the conditional gives right now, or nil when no
  clause matches.

### A click only a secure button may do

Some actions, such as running a slash command or clicking a protected
button, only work from a click on a secure button. `secure.overlay` lays a
secure button over a button of your own, so yours can stay an ordinary
frame. Make it out of combat:

```wowlua title=src/macro-button.wowlua
import * as secure from "wowlua:secure"

local myButton = CreateFrame("Button", nil, UIParent, "UIPanelButtonTemplate")

local overlay = secure.overlay(myButton)
overlay.set("type1", "macro")
overlay.set("macrotext1", "/click ExtraActionButton1")
overlay.show()                               -- false in combat
```

`set`, `show` and `hide` return false in combat, where they can't work.
`show(other)` moves the overlay over another frame instead, such as the row
of a list. `overlay.button` is the secure button, for `PreClick` and
`PostClick` hooks.

| Function | What it does |
|----------|--------------|
| `secure.visibility(frame, conditional)` | Shows and hides a frame by macro conditions |
| `secure.state(frame, name, conditional, fn?)` | A named state, with a function called on each change |
| `secure.evaluate(conditional)` | What a conditional gives right now |
| `secure.overlay(target, opts?)` | A secure button over one of yours |

## Run after Blizzard's functions

`@hook` runs your function after a global function or a method, as
`hooksecurefunc` does, without replacing it, so the original stays
untainted. Your function's parameters are checked against those of the
hooked function, so you get its arguments typed.

```wowlua title=src/hooks.wowlua
import hook, hookScript from "wowlua:hooks"

@hook("ToggleGameMenu")
local function afterGameMenu()
    print("game menu toggled")
end

@hook(GameTooltip, "SetUnit")
local function afterSetUnit(tip: GameTooltip)
    tip:AddLine("seen by MyAddon")
end

@hookScript(PlayerFrame, "OnShow")
local function onPlayerFrameShow(frame: Frame)
    frame:SetAlpha(0.9)
end
```

A target is a table or the name of a global, as a string; a string is looked
up when the hook is installed.

### Hooking a load-on-demand addon

Much of Blizzard's interface only loads when the player first opens it, so
the frame you want to hook doesn't exist at login. Name the addon, and the
hook waits for it:

```wowlua title=src/spellbook.wowlua
import hook from "wowlua:hooks"

@hook("PlayerSpellsFrame", "Show", { addon = "Blizzard_PlayerSpells" })
local function afterSpellbookShow(frame: Frame)
    print("spellbook opened", frame:GetName())
end
```

If you leave out `addon` for a target that lives in such an addon, wowlua
warns you when you build (W8102). Hooking a name the declarations don't know
is warning W8040.

Without decorators, `hook(target, name, fn)` and `hookScript(frame, script,
fn)` do the same as plain calls, and the function you write in the call gets
its parameter types from the target. `{ owner = … }` in the options is handed
to error listeners when the hook fails (see
[`wowlua:errors`](utils.md#when-something-goes-wrong)).

> [!WARNING]
> Hooks run after the original and can't be removed, as in the game itself.
> There are deliberately no pre-hooks and no replacements: replacing
> Blizzard's functions taints the default interface. Your hook is still
> addon code, so it follows the combat rules above.
