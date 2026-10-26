## In the emulator

`wowlua test` keeps a model of the player's bags: how many slots each bag has
and which item is in each slot, with its count and whether it is bound or
locked. The functions that read the bags (`GetContainerNumSlots`,
`GetContainerNumFreeSlots`, `GetContainerFreeSlots`, `GetContainerItemInfo`,
`GetContainerItemID`, `GetContainerItemLink`, `HasContainerItem`,
`CalculateTotalNumberOfFreeBagSlots`) answer from that model. The ones that
change the bags (`PickupContainerItem`, `SplitContainerItem`, `SortBags`,
`UseContainerItem`) update it and fire the events the client fires.

A camelot test that doesn't name a world profile starts with the fresh
character recorded from the client. Its backpack has 20 slots and holds Tough
Jerky and a Hearthstone. The keyring (bag -1) is always there, with 32 empty
slots. There are no equipped bags. `mock.setBag(bag, { slots, items })` or a
world profile's `bags` fills a bag, and `mock.setBag(bag, nil)` takes it away.
A missing bag has 0 slots, and `GetContainerNumFreeSlots` returns `0, nil` for
it.

The name, link, icon and quality of an item come from its item data: what the
test gave `mock.defineItem`. For an item the test doesn't define, the
emulator uses the data of your WoW installation when you run
`wowlua test --game-data`, else what wowlua recorded from the client for that
item, if anything.

The setters for the bag settings (sorting direction, insert order, autosort,
slot flags) accept their argument but don't keep it, so the getters keep
returning the same fixed answer.

`NUM_TOTAL_EQUIPPED_BAG_SLOTS` comes from Blizzard's UI code, which the
emulator only loads when the manifest sets `test.blizzard-ui`. Without it the
constant is `nil` in a test, so a loop up to it fails there. The examples on
these pages loop from `Enum.BagIndex.Backpack` to `Enum.BagIndex.ReagentBag`,
which works either way.

## Example

A `/bags` command that prints the free slots of each bag the player carries,
then the total.

```wowlua title=src/bags.wowlua
import slash from "wowlua:slash"
import print from "wowlua:log"

-- /bags: the free slots of each bag the player carries, then the total.
@slash("/bags")
local function listBags()
    for bag = Enum.BagIndex.Backpack, Enum.BagIndex.ReagentBag do
        local slots = C_Container.GetContainerNumSlots(bag)
        if slots > 0 then
            local free = C_Container.GetContainerNumFreeSlots(bag)
            print(`bag {bag}: {free} of {slots} free`)
        end
    end
    print(`{C_Container.CalculateTotalNumberOfFreeBagSlots()} slots free`)
end
```

`GetContainerNumSlots` returns 0 for a bag slot with no bag in it, so the loop
skips the empty ones.

## Testing

Set up the bags the test needs with `mock.setBag`. It replaces the whole bag,
including the default backpack. An item is an item ID, or a table such as
`{ id = 2589, count = 20 }` that can also set `link`, `bound` and `locked`.

```wowlua title=tests/bags.test.wowlua
import test, expect from "wowlua:test"
import * as mock from "wowlua:mock"

@test("/bags counts the free slots of each bag")
local function countsFreeSlots()
    mock.defineItem(2589, { name = "Linen Cloth", quality = 1 })
    mock.setBag(0, { slots = 16, items = { [1] = 2589, [2] = 2589 } })
    mock.setBag(1, { slots = 10, items = { [4] = { id = 2589, count = 20 } } })
    mock.slash("/bags")
    local shown = table.concat(mock.printed(), "\n")
    expect(shown).toContain("bag 0: 14 of 16 free")
    expect(shown).toContain("bag 1: 9 of 10 free")
    expect(shown).toContain("23 slots free")
end
```

`print` from `wowlua:log` puts the addon's name in front of each line, so the
test checks for the text with `toContain`.
