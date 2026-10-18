-- /stdtest engine scenario profession, played with the engine's controls:
-- the level 21 paladin mines a copper vein four times (scenarios/
-- profession.lua), opens Blacksmithing, makes a sharpening wheel, opens
-- Smelting. Each step at its recorded time (a click: a round trip before
-- the server's answer, ~0.35 s). Not yet: the filter's menu (29.8 s), a
-- recipe picked (43.1 s), the smelting (167 s on).
local e = __engine
local player = e.player
ScenarioPartial = "the smelting (167 s on) not played yet"
e.setSpellTraits({ offGlobalCooldown = { 20271 }, tracking = { 2580, 5502 }, profession = { 3100, 2656 } })
-- The spell and the stone as the client named them (UnitCastingInfo, the
-- loot's link and icon).
e.defineSpell(2576, { name = "Mining", iconID = 136248 })
e.defineItem(2835, { name = "Rough Stone", quality = 1, icon = 135232, stackCount = 20, classID = 7, subclassID = 7 }, { cached = true })
local vein = e.spawnUnit({ name = "Copper Vein", guid = "GameObject-0-6783-1-5-1731-E4B5B5093C", reaction = "friendly" })
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
local function world(...) e.fireEvent(...) end
e.setLatency(0.345)
-- The vein mined four times: a cast of 3.208 s, its ore autolooted (the
-- third time with two rough stones).
local ore = { { id = 2770 } }
at(2.872) player:softTarget(vein, "interact")
at(3.504) world("PLAYER_STOPPED_MOVING")
at(3.734) player:gather(vein, { spell = 2576, castTime = 3.208, loot = ore, auto = true })
at(9.672) player:gather(vein, { spell = 2576, castTime = 3.208, loot = ore, auto = true })
at(14.495) player:gather(vein, { spell = 2576, castTime = 3.208, loot = { { id = 2770 }, { id = 2835, count = 2 } }, auto = true })
at(19.363) player:gather(vein, { spell = 2576, castTime = 3.208, loot = ore, auto = true })
at(24.2)
check("the ore in the bag", C_Item.GetItemCount(2770) == 18)
-- Blacksmithing's spell clicked on the bar: its window.
at(25.395) player:openTradeSkill(3100, "Blacksmithing")
at(25.561) world("PLAYER_STARTED_MOVING")
at(26.201) world("PLAYER_STOPPED_MOVING")
at(27.0)
check("the window shown", ProfessionsFrame and ProfessionsFrame:IsShown())
check("the wheel's recipe shown", ProfessionsFrame.CraftingPage.SchematicForm:GetRecipeInfo().recipeID == 1230171)
local function press(frame, button)
    if not frame then
        check("clicked: no frame", false)
        return
    end
    local ok, err = pcall(e.click, frame, button)
    if not ok then check("clicked: " .. tostring(err), false) end
end
-- The wheel made: a cast of 10 s; the window closed meanwhile.
at(27.640) press(ProfessionsFrame.CraftingPage.CreateButton)
at(37.442) e.pressKey("ESCAPE")
-- The spellbook, Blacksmithing again, Smelting.
at(37.985) e.pressKey("P")
at(39.0)
check("the wheel made", C_Item.GetItemCount(279944) == 2)
at(39.546) player:openTradeSkill(3100, "Blacksmithing")
at(41.004) player:openTradeSkill(2656, "Mining")
at(45.634) world("PLAYER_STARTED_TURNING")
at(46.065) world("PLAYER_STARTED_MOVING")
at(47.742) e.pressKey("ESCAPE")
at(48.0)
