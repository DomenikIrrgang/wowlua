-- /stdtest engine scenario bags, played with the engine's controls: the
-- level 21 paladin in the Crossroads opens all bags (Shift-B); splits its
-- six Blood Shards with a shift-click, three into an empty backpack slot;
-- drags its Rough Stone onto bag 1's Sharpening Wheel (a swap) and the
-- Mining Pick from bag 1 into the backpack's last slot; sorts the bags
-- (scenarios/bags.lua). Each step at its recorded time (a click's press or
-- release as the client acted on it), each answer's round trip as
-- recorded.
local e = __engine
local player = e.player
e.setSpellTraits({ offGlobalCooldown = { 20271 }, tracking = { 2580, 5502 }, profession = { 3100, 2656 }, partyOrSelf = { 1022 } })
local now = 0
-- Time passes in frames, as in the client (60 a second: Blizzard's
-- animations and OnUpdate code run), the rest of a step as one jump.
local function at(t)
    if t < now then error(("at(%s): before %s"):format(t, now), 2) end
    local frames = math.floor((t - now) * 60)
    if frames > 0 then
        e.advanceFrames(frames, 1000 / 60)
    end
    e.advanceTime(math.max(0, t - now - frames / 60))
    now = t
end
local function press(frame, button)
    local ok, err = pcall(e.click, frame, button)
    if not ok then check("clicked: " .. tostring(err), false) end
end
local function run(frame, script, ...)
    local ok, err = pcall(e.script, frame, script, ...)
    if not ok then check(script .. ": " .. tostring(err), false) end
end
-- A bag slot's button.
local function bagButton(bag, slot)
    for _, name in ipairs({ "ContainerFrameCombinedBags", "ContainerFrame1", "ContainerFrame2", "ContainerFrame3", "ContainerFrame4", "ContainerFrame5", "ContainerFrame6" }) do
        local f = _G[name]
        if f and f:IsShown() and f.EnumerateValidItems then
            for _, b in f:EnumerateValidItems() do
                if b:GetBagID() == bag and b:GetID() == slot then return b end
            end
        end
    end
end
-- The world's own, not modelled: the server's events as recorded.
local function world(...) e.fireEvent(...) end
-- This session's server told the bags' updates bag 1 first, then the
-- reagent bag, then the backpack (the moves' and the sort's packets).
e.setBagOrder({ 1, 5, 0 })
-- The character's bank tabs (the bank closed; the sort locked the first
-- tab's items too): as the bank scenario left them (its ore moved to slot
-- 7: the slots the sort locked).
e.defineBank(e.spawnUnit({ name = "Banker", reaction = "friendly" }), {
    tabs = {
        { name = "Tab 1", icon = 134400, slots = 48, items = {
            [1] = { id = 1210, count = 1 }, [2] = { id = 249391, count = 9 }, [3] = { id = 2771, count = 5 },
            [4] = { id = 3470, count = 20 }, [5] = { id = 2840, count = 20 }, [6] = { id = 2840, count = 20 },
            [7] = { id = 2770, count = 12 }, [9] = { id = 2835, count = 20 }, [10] = { id = 2835, count = 2 },
        } },
        { name = "Tab 2", icon = 134400, slots = 48 },
        { name = "Tab 3", icon = 134400, slots = 48 },
    },
})
at(4.009) e.pressKey("SHIFT-B")
-- Tooltips of the items hovered (the client's own).
at(7.366) world("TOOLTIP_DATA_UPDATE", 4245)
at(7.4)
check("the bags open", bagButton(0, 1) ~= nil and bagButton(1, 1) ~= nil)
-- The Blood Shards split: the stack's split frame (a shift-click), three
-- taken (its Okay), put into the empty slot 19 (the server's answer 0.421 s
-- later, the new stack's data 0.299 s after that).
at(8.193)
e.setModifiers({ shift = true })
press(bagButton(0, 15))
e.setModifiers({})
at(8.6)
check("the split frame", StackSplitFrame:IsShown())
if StackSplitFrame:IsShown() then StackSplitFrame.split = 3 end
at(8.844) press(StackSplitFrame.OkayButton)
check("three shards held", select(1, GetCursorInfo()) == "item")
e.setLatency(0.421)
e.setItemDataDelay(0.299)
at(9.419) press(bagButton(0, 19))
at(10.5)
check("split: three and three", C_Container.GetContainerItemInfo(0, 15).stackCount == 3 and C_Container.GetContainerItemInfo(0, 19).stackCount == 3)
-- The Rough Stone dragged onto bag 1's Sharpening Wheel: swapped 0.468 s
-- after the drop.
at(16.074) run(bagButton(0, 1), "OnDragStart", "LeftButton")
at(16.336) world("TOOLTIP_DATA_UPDATE", 4269)
e.setLatency(0.468)
at(16.487) run(bagButton(1, 3), "OnReceiveDrag")
at(17.5)
check("swapped", C_Container.GetContainerItemID(0, 1) == 279944 and C_Container.GetContainerItemID(1, 3) == 2835)
-- The Mining Pick into the backpack's last slot (0.318 s).
at(18.446) run(bagButton(1, 4), "OnDragStart", "LeftButton")
at(18.544) world("TOOLTIP_DATA_UPDATE", 4295)
e.setLatency(0.318)
at(19.048) run(bagButton(0, 20), "OnReceiveDrag")
at(19.5)
check("moved", C_Container.GetContainerItemID(0, 20) == 2901 and C_Container.GetContainerItemID(1, 4) == nil)
at(21.997) world("UPDATE_INVENTORY_DURABILITY")
-- Sorted (0.434 s): where the server put everything (the after
-- snapshot's bags), the two Blood Shard stacks merged in bag 1's second
-- slot, its six told 0.181 s after the sort's packet.
local function items(list)
    local out = {}
    for slot, v in pairs(list) do out[slot] = { id = v[1], count = v[2] or 1, shownFirst = v[3], after = v[4] } end
    return out
end
e.setSortResult({
    [0] = items({ { 6948 }, { 279968 }, { 279944, 2 }, { 2901 }, { 5956 }, { 4471 }, { 2770, 13 }, { 2835 },
        { 251382 }, { 4238 }, { 4238 }, { 277115 }, { 277566 }, { 3317 }, { 286176 }, { 267454 }, { 16603 },
        { 14544 }, { 10649 }, { 6443, 13 } }),
    [1] = items({ { 5088 }, { 5075, 6, 3, 0.181 }, { 8067 } }),
    [5] = items({ { 248685 }, { 248682 }, { 2841, 8 }, { 2771, 3 }, { 2836, 9 }, { 2840 } }),
})
e.setLatency(0.434)
at(22.626) press(BagItemAutoSortButton)
at(24)
check("sorted", C_Container.GetContainerItemID(0, 1) == 6948 and C_Container.GetContainerItemID(1, 2) == 5075)
at(33.9)
