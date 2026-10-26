## When it fires

When the bags change, the client fires these events in this order:

1. `BAG_CLOSED(bag)` for a bag that was taken away.
2. `BAG_UPDATE(bag)` for the bags that changed. Every `BAG_UPDATE(0)` for the
   backpack is followed by `BAG_UPDATE(-1)`, the keyring's index
   (`Enum.BagIndex.Keyring`), even when the keyring didn't change.
3. `BAG_CONTAINER_UPDATE` when an equipped bag was put in or taken away.
4. `BAG_UPDATE_DELAYED`, once, after the bags' updates.
5. `ITEM_COUNT_CHANGED(itemID)` for the items whose count changed.

`BAG_UPDATE` can fire more than once for the same bag. A loot fires it once
for each item placed, and the login fires `BAG_UPDATE(0)` and
`BAG_UPDATE(-1)` once for each item the character has.

After a loot, `ITEM_COUNT_CHANGED` fires only for the stacks that grew, not
for an item that went into an empty slot. That slot is taken at once, so
`GetContainerNumFreeSlots` no longer counts it as free, but
`GetContainerItemInfo` reads it as empty until the item's data arrives a
moment later. Then `BAG_UPDATE` and
`BAG_UPDATE_DELAYED` fire again, and the slot reads the item. An addon that
rescans on every `BAG_UPDATE_DELAYED` sees the new item on the second one.

## In the emulator

The emulator fires these events in the order above, which was recorded from
the client. The one step that wasn't recorded is `BAG_CONTAINER_UPDATE` for a
bag put in, which follows Blizzard's documentation and UI code. The events
fire when items are looted, bought or sold at a vendor, taken from
or sent with the mail, posted at the auction house, crafted, moved, split or
sorted, equipped from the bags, used up or destroyed, and given as quest
rewards. They also fire at login and when a test calls `mock.setBag`.

`mock.setBag` fires `BAG_UPDATE` and `BAG_UPDATE_DELAYED` even when the bag's
contents stay the same. After them, `ITEM_COUNT_CHANGED` fires for every item
whose total count in the bags changed, in item ID order.

By default, a test sees each event as soon as the change fires it. After
`mock.setFrameBatching(true)`, `BAG_UPDATE_DELAYED` and `ITEM_COUNT_CHANGED`
wait for the end of the frame, as they do in the client. Then several changes
in one frame share a single `BAG_UPDATE_DELAYED`, and each item's
`ITEM_COUNT_CHANGED` fires once.

## Example

Note which bags changed on `BAG_UPDATE`, and do the work once on
`BAG_UPDATE_DELAYED`.

```wowlua title=src/bags.wowlua
import event from "wowlua:events"

-- The free slots of each bag, kept up to date.
export local freeByBag: { [number]: number } = {}

local changed: { [number]: boolean } = {}

@event("BAG_UPDATE")
local function bagChanged(bagID: BagIndex)
    changed[bagID] = true
end

@event("BAG_UPDATE_DELAYED")
local function bagsSettled()
    for bag in pairs(changed) do
        freeByBag[bag] = C_Container.GetContainerNumFreeSlots(bag)
    end
    changed = {}
end
```

`@event` registers the handlers and checks their parameters against the
event's payload. A misspelt event name, such as `@event("BAG_UPDATED")`, is a
compile error.

## Coalescing a burst

If all you need to know is that the bags changed, `@bucket` from
`wowlua:events` collects a burst of events into one call. It waits for the
interval after the first event, then passes the counts, with
`counts.args` keyed by the bag.

```wowlua title=src/bags.wowlua
import bucket from "wowlua:events"
import type BucketCounts from "wowlua:events"

-- The free slots of each bag, at most one update every 0.2 seconds.
export local freeByBag: { [number]: number } = {}

@bucket(0.2, "BAG_UPDATE")
local function bagsChanged(counts: BucketCounts)
    for bag in pairs(counts.args) do
        freeByBag[bag] = C_Container.GetContainerNumFreeSlots(bag)
    end
end
```

## Testing

`mock.setBag` changes a bag and fires the events as the client would.
`mock.fireEvent` fires a single event. Its payload is checked against the
documentation, so leaving out the bag, passing an extra value or a bag that
isn't a number is an error.

```wowlua title=tests/bags.test.wowlua
import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"
import freeByBag from "/bags"

@test("follows the free slots of a bag")
local function followsBag()
    mock.setBag(1, { slots = 10, items = { [1] = 2589, [2] = 2589 } })
    expect(freeByBag[1]).toBe(8)
    mock.setBag(1, nil)
    expect(freeByBag[1]).toBe(0)
end

@test("a bare BAG_UPDATE waits for BAG_UPDATE_DELAYED")
local function waitsForDelayed()
    mock.setBag(2, { slots = 6 })
    freeByBag[2] = nil
    mock.fireEvent("BAG_UPDATE", 2)
    expect(freeByBag[2]).toBeNil()
    mock.fireEvent("BAG_UPDATE_DELAYED")
    expect(freeByBag[2]).toBe(6)
end
```

To test the `@bucket` version, move the clock past the interval with
`mock.advanceTime`:

```wowlua title=tests/bags.test.wowlua
import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"
import freeByBag from "/bags"

@test("updates once the burst is over")
local function waitsForTheBurst()
    mock.setBag(1, { slots = 10, items = { [1] = 2589 } })
    mock.setBag(2, { slots = 12 })
    expect(freeByBag[1]).toBeNil()
    mock.advanceTime(0.2)
    expect(freeByBag[1]).toBe(9)
    expect(freeByBag[2]).toBe(12)
end
```
