## In the emulator

The emulator answers from its model of the bags and from the item's data:

- `stackCount`, `isBound` and `isLocked` come from the slot, as
  `mock.setBag` or a world profile filled it (`count`, `bound`, `locked`).
  `isLocked` is also true while the item is on the cursor, after
  `C_Container.PickupContainerItem`.
- `itemName`, `iconFileID` and `quality` come from the item's data
  (`mock.defineItem(id, { name, icon, quality })`), and `isReadable` from its
  `readable`.
- `hasNoValue` is true when the item's `sellPrice` is 0, as for the
  Hearthstone. An item whose data has no `sellPrice` has `hasNoValue = false`.
- `hyperlink` is the slot's own `link` if the test gave one, else the data's
  `link`, else a link built the way the client builds one, from the item's
  name, its quality and the player's level.
- `hasLoot` and `isFiltered` are always false.

It returns `nil` for an empty slot and for a bag that isn't there. After a
loot, an item that went into an empty slot reads as `nil` until its data
arrives a moment later, as in the client (see `BAG_UPDATE`).

An item the test doesn't define, and that wowlua has no data for, has `nil`
for `itemName`, `hyperlink`, `iconFileID` and `quality`, although its type
says they are always there. Define every item your test puts in the bags.

## Example

A `/junk` command that prints the links of the grey items in the bags that a
vendor would buy.

```wowlua title=src/junk.wowlua
import slash from "wowlua:slash"
import print from "wowlua:log"

-- The links of the grey items in the bags that a vendor would buy.
export function junk(): string[]
    local links: string[] = {}
    local poor = Enum.ItemQuality.Poor
    for bag = Enum.BagIndex.Backpack, Enum.BagIndex.ReagentBag do
        for slot = 1, C_Container.GetContainerNumSlots(bag) do
            local info = C_Container.GetContainerItemInfo(bag, slot)
            if info and info.quality == poor and not info.hasNoValue then
                table.insert(links, info.hyperlink)
            end
        end
    end
    return links
end

@slash("/junk")
local function listJunk()
    local links = junk()
    if #links == 0 then
        print("no junk")
    else
        print(`junk: {table.concat(links, " ")}`)
    end
end
```

The result is `ContainerItemInfo?`, so check it before reading its fields. In
a `--!strict` file, reading `info.quality` without the `info and` check is
error E2005. A misspelt field, such as `info.hyperLink`, is error E2003 in
every mode.

## Testing

Define the items, fill a bag with them, then run the command.

```wowlua title=tests/junk.test.wowlua
import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"

@test("/junk lists the grey items a vendor buys")
local function listsJunk()
    mock.defineItem(4865, { name = "Ruined Pelt", quality = 0, sellPrice = 5 })
    mock.defineItem(2589, { name = "Linen Cloth", quality = 1, sellPrice = 13 })
    mock.defineItem(90001, { name = "Lucky Stone", quality = 0, sellPrice = 0 })
    mock.setBag(0, {
        slots = 16,
        items = { [1] = { id = 4865, count = 3 }, [2] = 2589, [3] = 90001 },
    })
    mock.slash("/junk")
    local shown = table.concat(mock.printed(), "\n")
    expect(shown).toContain("[Ruined Pelt]")
    expect(shown).never.toContain("Linen Cloth")
    expect(shown).never.toContain("Lucky Stone")
end

@test("an empty slot has no info")
local function emptySlot()
    mock.setBag(0, { slots = 16 })
    expect(C_Container.GetContainerItemInfo(0, 1)).toBeNil()
    expect(C_Container.GetContainerItemInfo(3, 1)).toBeNil()
end
```

The Lucky Stone is grey too, but its `sellPrice` of 0 makes `hasNoValue`
true, so `/junk` leaves it out.
