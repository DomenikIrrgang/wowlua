# Events and lifecycle

`wowlua:events` handles WoW events, unit events and messages your addon sends
to itself. `wowlua:lifecycle` runs your code when the addon loads, the player
logs in or out, and another addon becomes available.

## Handle a WoW event

`@event` makes a function the handler of an event. Its parameters are the
event's payload, typed from the game's API, so the editor completes them and
the compiler checks how you use them. The
[WoW API reference](../reference/api.md) lists every event and its payload.

```wowlua title=src/zone.wowlua
import event from "wowlua:events"
import print from "wowlua:log"

@event("PLAYER_ENTERING_WORLD")
local function enteredWorld(isInitialLogin: boolean, isReloadingUi: boolean)
    if isInitialLogin or isReloadingUi then
        print("logged in at", GetZoneText())
    end
end

@event("ZONE_CHANGED_NEW_AREA")
local function zoneChanged()
    print("entered", GetZoneText())
end
```

The module keeps one hidden frame for these handlers. It registers an event
with the game while at least one handler wants it and unregisters it when the
last one is gone. There is no `OnEvent` script to write.

> [!TIP]
> Event names are checked against the game's API. When a patch removes or
> renames an event, the build fails with E8003 instead of the handler silently
> never running.

## One handler for several events

Give `@event` several names and the handler receives the event's name first,
then the payload as `...: any`. The name's type is the union of the names you
listed, so a comparison with a name you didn't list is caught.

```wowlua title=src/bags.wowlua
import event from "wowlua:events"
import print from "wowlua:log"

@event("BAG_UPDATE", "BAG_UPDATE_DELAYED")
local function bagsChanged(e: "BAG_UPDATE" | "BAG_UPDATE_DELAYED", ...: any)
    if e == "BAG_UPDATE_DELAYED" then
        print("free slots:", C_Container.GetContainerNumFreeSlots(0))
    end
end
```

The names may mix WoW events and your own [messages](#send-your-own-messages).

## Only listen to some units

`@unitEvent` registers a unit event for the units you name and no others. A
party frame no longer runs for every nameplate's health change.

```wowlua title=src/party.wowlua
import unitEvent from "wowlua:events"
import print from "wowlua:log"

@unitEvent("UNIT_HEALTH", "player", "party1", "party2", "party3", "party4")
local function healthChanged(unit: UnitToken)
    print(unit, UnitHealth(unit), "/", UnitHealthMax(unit))
end
```

The game filters a frame's unit event by at most two units, and a plain
`RegisterEvent` of the same event on that frame would clear the filter. So each
`@unitEvent` gets hidden frames of its own, one per pair of units. Unit filters
never interfere with each other or with `@event` handlers of the same event.

## Turn handlers on and off

Decorated handlers live as long as the addon. For handlers that come and go,
use `on`, `once` and `off`. `on` and `once` return a `Subscription`, which you
cancel with `off(sub)` or `sub:cancel()`.

```wowlua title=src/window.wowlua
import on, once, off from "wowlua:events"

local window = CreateFrame("Frame", nil, UIParent)

-- hide the window for the length of a fight
local hide = on("PLAYER_REGEN_DISABLED", function()
    window:Hide()
end)
local show = on("PLAYER_REGEN_ENABLED", function()
    window:Show()
end)

-- runs on the first login only, then unregisters itself
once("PLAYER_LOGIN", function()
    window:SetPoint("CENTER")
end)

export function stayVisible()
    off(hide)
    show:cancel()
end
```

An instance method can't carry `@event`: the decorator runs once, when the
class is defined, and there may be many objects (E8002). Register in the
constructor instead and pass the object as the third argument, the owner.
`offAll(owner)` then cancels everything that object registered.

```wowlua title=src/tracker.wowlua
import on, offAll from "wowlua:events"
import print from "wowlua:log"

export class FightTimer
    started: number = 0

    constructor()
        on("PLAYER_REGEN_DISABLED", function() self:enter() end, self)
        on("PLAYER_REGEN_ENABLED", function() self:leave() end, self)
    end

    function enter()
        self.started = GetTime()
    end

    function leave()
        print(`the fight took {GetTime() - self.started} seconds`)
    end

    function discard()
        offAll(self)
    end
end
```

`offAll(owner, name)` cancels only that owner's handlers of one event.
`offAll(nil)` is an error: it would cancel every subscription without an owner,
the standard library's own included.

`onUnit(event, units, handler, owner?)` is the dynamic form of `@unitEvent`:
one subscription for all its units.

## Follow a unit that keeps changing

A nameplate or a unit frame shows one unit at a time, and the unit changes.
`watchUnit` registers a set of unit events on a frame of its own. `setUnit`
points it at another unit without subscribing again.

```wowlua title=src/plate.wowlua
import watchUnit from "wowlua:events"
import print from "wowlua:log"

local watch = watchUnit({ "UNIT_HEALTH", "UNIT_MAXHEALTH" }, function(e: "UNIT_HEALTH" | "UNIT_MAXHEALTH", unit: UnitToken)
    print(e, unit, UnitHealth(unit))
end)

watch:setUnit("nameplate3")     -- follows nameplate3
watch:setUnit("nameplate5")     -- now nameplate5 instead
watch:setUnit(nil)              -- listens to no unit
watch:cancel()                  -- done for good
```

The handler gets the event name first, then the payload. `setUnit` takes a
second unit too, as the game's unit filter does. A cancelled watch's frame is
reused by the next watch.

## Calm down noisy events

Some events come in bursts: `BAG_UPDATE` once per bag when you loot,
`UNIT_HEALTH` many times a second in a fight. `@bucket(interval, ...)` starts
a wait of `interval` seconds at the first event of a burst, then calls the
handler once with counts of what arrived.

```wowlua title=src/loot.wowlua
import bucket from "wowlua:events"
import type BucketCounts from "wowlua:events"
import print from "wowlua:log"

@bucket(0.5, "BAG_UPDATE")
local function bagsSettled(counts: BucketCounts)
    for bag, times in pairs(counts.args) do
        print("bag", bag, "changed", times, "times")
    end
end
```

`counts.args` counts by the events' first argument (the bag of `BAG_UPDATE`,
the unit of a unit event), `counts.events` by event name. A burst longer than
the interval gives one call per interval. `onBucket(events, interval, handler)`
does the same without a decorator and returns a subscription.

## Send your own messages

A message is an event your addon makes up, so that one part of it can tell the
others that something happened. Declare it with its payload, handle it with
`@event` or `on`, and send it with `send`. Messages are dispatched in Lua and
never registered with the game.

```wowlua title=src/scale.wowlua
import event, send from "wowlua:events"

declare message MYADDON_SCALE_CHANGED(scale: number)

local window = CreateFrame("Frame", nil, UIParent)

@event("MYADDON_SCALE_CHANGED")
local function rescale(scale: number)
    window:SetScale(scale)
end

export function setScale(scale: number)
    send("MYADDON_SCALE_CHANGED", scale)
end
```

`send` is checked against the declaration, so a missing argument or one of the
wrong type is a compile error, and so is sending a WoW event (E8001). Delivery
is immediate and reaches only your own addon's handlers. A name declared both
as an event and as a message is E2058.

## Run code at the right moment of startup

`wowlua:lifecycle` has a decorator for each moment of the startup sequence an
addon cares about, so no `ADDON_LOADED` handler has to check its own name.

```wowlua title=src/main.wowlua
import onLoad, onLogin, onEnterWorld, onLogout from "wowlua:lifecycle"
import print from "wowlua:log"

@onLoad
local function init()
    -- this addon's SavedVariables are loaded
    print("loaded")
end

@onLogin
local function start()
    -- the UI and the character's data are there
    print("hello,", UnitName("player"))
end

@onEnterWorld
local function entered(isInitialLogin: boolean, isReloadingUi: boolean)
    -- after every loading screen
    print("loading screen done", isInitialLogin, isReloadingUi)
end

@onLogout
local function bye()
    -- the last chance to change saved data
end
```

- `@onLoad` runs on `ADDON_LOADED` for your own addon. Just before it,
  [`wowlua:saved`](saved.md) loads, migrates and attaches defaults to every
  bound SavedVariable, so they are usable here.
- `@onLogin` runs on `PLAYER_LOGIN`. A load-on-demand addon that loads after
  login gets it right after its `@onLoad` hooks.
- `@onEnterWorld` runs on every `PLAYER_ENTERING_WORLD`, with the event's two
  flags.
- `@onLogout` runs on `PLAYER_LOGOUT`, before `wowlua:saved` strips values
  equal to their defaults, so what you write here is saved.

Hooks of one kind run in load order: files in the order they load, then top to
bottom within a file. A hook that fails is reported, and the others still run.

The standard library does its own setup around your hooks. The
[options page](settings.md) is created after the `@onLoad` hooks, so it sees
the values they fixed up. The minimap button and the Addon Compartment entry
are created at login, after the `@onLogin` hooks.

## Wait for another addon

Many of Blizzard's panels are addons that load on demand. `onAddonLoaded`
runs a function once that addon has loaded, or right away if it already has.

```wowlua title=src/spellbook.wowlua
import onAddonLoaded, isLoggedIn from "wowlua:lifecycle"
import print from "wowlua:log"

onAddonLoaded("Blizzard_PlayerSpells", function()
    print("the talent and spellbook panel is loaded")
end)

if isLoggedIn() then
    print("this file ran after login")
end
```

`isLoaded()` tells whether your addon's `ADDON_LOADED` has fired, and
`isLoggedIn()` whether `PLAYER_LOGIN` has, or the addon loaded after it.

> [!NOTE]
> Every handler on this page runs isolated. An error goes to the game's error
> handler with your addon's name in front, so error display addons such as
> BugSack catch it, and the remaining handlers of that event still run. The
> rules are in [`wowlua:errors`](combat.md).

The full rules of both modules are in the specification:
[events](../../spec/stdlib/core.md#events) and
[lifecycle](../../spec/stdlib/core.md#lifecycle).
