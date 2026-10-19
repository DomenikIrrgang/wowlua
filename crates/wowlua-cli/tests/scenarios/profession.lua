-- /stdtest engine scenario profession, played with the engine's controls:
-- the level 21 paladin mines a copper vein four times (scenarios/
-- profession.lua), opens Blacksmithing, makes a sharpening wheel, opens
-- Smelting; walks to a forge (not played: 48-167 s), smelts copper bars,
-- clicks tin's Create again and again while they're made, smelts tin bars
-- (moving stops it) and bronze bars (three skill-ups). Each step at its
-- recorded time (a click: a round trip before the server's answer,
-- ~0.35 s). Not yet: the filter's menu (29.8 s), a recipe picked (43.1 s),
-- the walk; the Create clicks the client ignored (no
-- TRADE_SKILL_CRAFT_BEGIN: a request still unanswered) are left out.
local e = __engine
local player = e.player
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
-- At the forge: Smelting again; bronze and copper picked.
player:setSpellFocus({ "Forge" })
local function pick(recipe)
    local list = ProfessionsFrame.CraftingPage.RecipeList
    press(list.ScrollBox:FindFrameByPredicate(function(_, node)
        local data = node:GetData()
        return data.recipeInfo ~= nil and data.recipeInfo.recipeID == recipe
    end))
end
local page = ProfessionsFrame and ProfessionsFrame.CraftingPage
at(167.288) player:openTradeSkill(2656, "Mining")
at(170.091) pick(2659)
at(171.114) pick(2657)
check("copper picked", page.SchematicForm:GetRecipeInfo().recipeID == 2657)
-- Every copper bar: casts of 3 s, one after another (each next cast
-- ~0.3 s after the last one's success: the ping there).
e.setLatency(0.3)
at(188.866) press(page.CreateAllButton)
-- Tin picked; its Create clicked while copper is made: refused ("Another
-- action is in progress"), the last ones queued, one made.
at(196.218) pick(3304)
for _, t in ipairs({ 198.878, 200.080, 200.511, 201.009, 201.578, 201.732, 202.032, 202.208 }) do
    at(t) press(page.CreateButton)
end
at(208.993) press(page.CreateAllButton)
-- Bronze picked while tin is made; moving stops the tin.
at(210.715) pick(2659)
player:craftSkillUps(2659, { false, true, true, true })
at(219.122) world("PLAYER_STARTED_MOVING") player:interruptCast()
at(219.249) world("PLAYER_STOPPED_MOVING")
at(219.590) press(page.CreateAllButton)
at(234.0)
check("the bronze made", C_Item.GetItemCount(2841) == 8)
at(235.277) e.pressKey("ESCAPE")
at(236.0)
