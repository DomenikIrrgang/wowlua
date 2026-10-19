-- /stdtest engine scenario profession, played with the engine's controls:
-- the level 21 paladin mines a copper vein four times (scenarios/
-- profession.lua), opens Blacksmithing, makes a sharpening wheel, opens
-- Smelting; walks to a forge in the Crossroads (48-167 s: the quest log's
-- updates, mobs soft targeted, Holy Light pressed while moving, NPCs
-- soft targeted), smelts copper bars, clicks tin's Create again and again
-- while they're made, smelts tin bars (moving stops it) and bronze bars
-- (three skill-ups). Each step at its recorded time (a click: a round
-- trip before the server's answer, ~0.35 s). Not yet: the filter's menu
-- (29.8 s: which entry, unrecorded); the Create clicks the client ignored
-- (no TRADE_SKILL_CRAFT_BEGIN: a request still unanswered) are left out.
local e = __engine
-- The Sharpening Wheel's craft may be interrupted (recorded: its
-- UnitCastingInfo's eighth false; the other crafts' true).
e.setSpellTraits({ interruptible = { 1230171 } })
local player = e.player
e.setSpellTraits({ offGlobalCooldown = { 20271 }, tracking = { 2580, 5502 }, profession = { 3100, 2656 }, partyOrSelf = { 1022 } })
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
-- The vein mined four times: a cast of 3.2 s, its ore autolooted (the
-- third time with two rough stones). Each cast's start as recorded (the
-- server's answer 0.364, 0.378, 0.340 and 0.348 s after the click): a
-- moment of its own, at that time; its success, the slots' answers after
-- the window opened and each item's bags' packet after it as recorded.
local ore = { { id = 2770 } }
local function mine(t, answer, succeedsAfter, loot, slotsAfter, bagsAfter)
    e.setLatency(answer)
    at(t) player:gather(vein, {
        spell = 2576, castTime = 3.2, loot = loot, auto = true, succeedsAfter = succeedsAfter,
        slotsAfter = slotsAfter, bagsAfter = bagsAfter,
    })
    at(t + answer + 0.001)
    e.setLatency(0.345)
end
at(2.872) player:softTarget(vein, "interact")
at(3.504) world("PLAYER_STOPPED_MOVING")
mine(3.734, 0.364, 3.208, ore, { 0.566 }, { 0.334 })
mine(9.672, 0.378, 3.154, ore, { 0.605 }, { 0.405 })
mine(14.495, 0.340, 3.197, { { id = 2770 }, { id = 2835, count = 2 } }, { 0.517, 0.713 }, { 0.160, 0.036 })
mine(19.363, 0.348, 3.192, ore, { 0.528 }, { 0.061 })
at(24.2)
check("the ore in the bag", C_Item.GetItemCount(2770) == 18)
-- Blacksmithing's spell clicked on the bar: its window.
at(25.395) player:openTradeSkill(3100, "Blacksmithing")
at(25.561) player:setMoving(true)
at(26.201) player:setMoving(false)
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
local function pick(recipe)
    local list = ProfessionsFrame.CraftingPage.RecipeList
    press(list.ScrollBox:FindFrameByPredicate(function(_, node)
        local data = node:GetData()
        return data.recipeInfo ~= nil and data.recipeInfo.recipeID == recipe
    end))
end
-- The wheel made: a cast of 10 s; the window closed meanwhile.
-- Its cast 0.343 s after the click (recorded).
e.setLatency(0.343)
at(27.640) press(ProfessionsFrame.CraftingPage.CreateButton)
at(27.984) e.setLatency(0.345)
-- The window's list updated mid-cast, its cause unknown (recorded).
at(31.474) world("TRADE_SKILL_LIST_UPDATE")
at(37.442) e.pressKey("ESCAPE")
-- The spellbook, Blacksmithing again, Smelting.
at(37.985) e.pressKey("P")
at(39.0)
check("the wheel made", C_Item.GetItemCount(279944) == 2)
at(39.546) player:openTradeSkill(3100, "Blacksmithing")
at(41.004) player:openTradeSkill(2656, "Mining")
at(43.197) pick(2657)
at(45.634) world("PLAYER_STARTED_TURNING")
at(46.065) player:setMoving(true)
at(47.742) e.pressKey("ESCAPE")
-- The walk to the forge (48-167 s), the world's own as recorded: the
-- server's quest log updates (the log the character's, recorded before),
-- its quest lines'.
local function questLog() world("QUEST_LOG_UPDATE") end
-- Mobs near the road soft-targeted and lost again, with no target of the
-- player's: the server told of each (`UNIT_TARGET("player")` alone ~0.5 s
-- after the soft target, as on the taxi flight); each lost one a
-- `PLAYER_TARGET_CHANGED` with no target, the server's echo a round trip
-- later (recorded; unverified why: the soft targeting itself not modelled).
local function servers() world("UNIT_TARGET", "player") end
local function lost() player:target(nil) end
at(48.311) questLog()
at(48.664) world("QUESTLINE_UPDATE", false)
at(51.697) servers()
at(51.933) lost()
at(52.948) servers()
at(55.348) lost()
at(59.725) lost()
at(59.857) servers()
at(73.409) questLog()
at(87.050) questLog()
at(87.465) world("QUESTLINE_UPDATE", false)
-- Friends going offline and coming online (the server's messages).
at(91.244) e.systemMessage("Otherplayer has gone offline.")
at(97.828) questLog()
at(101.034) e.systemMessage("|Hplayer:Player Lastname|h[Player Lastname]|h has come online.")
at(109.202) servers()
at(111.582) lost()
-- Holy Light pressed on the way (its key, Cmd-2): refused, moving.
at(117.918) player:useSpell(647)
at(124.942) servers()
at(131.308) lost()
at(133.875) servers()
at(135.708) lost()
-- On the Gold Road, into the Crossroads (the recording's zone texts).
at(138.575) e.moveTo({ zone = "The Barrens", subZone = "Gold Road", mapID = 1413 })
-- The quest watch list told after the area's updates (recorded, in their
-- frame; why unverified).
at(138.625) world("QUEST_WATCH_LIST_CHANGED", nil, nil)
at(142.908) e.moveTo({ zone = "The Barrens", subZone = "The Crossroads", mapID = 1413 })
-- The Crossroads' NPCs soft targeted (who, unrecorded: the soft friend's
-- events alone tell): one, then a caster spending mana (its spell and
-- power secret or unasked: any), then others.
local npc = e.spawnUnit({ name = "Crossroads Grunt", reaction = "friendly" })
local caster = e.spawnUnit({ name = "Crossroads Caster", reaction = "friendly", powerType = 0, power = 1000, maxPower = 1000 })
local other = e.spawnUnit({ name = "Crossroads Peon", reaction = "friendly" })
at(145.408) player:softTarget(npc, "friend")
-- An aura gone from it, ENCHANT_SPELL_COMPLETED (its cause unknown): the
-- world's, as recorded.
at(145.608) world("UNIT_AURA", "softfriend", { isFullUpdate = false, removedAuraInstanceIDs = { 1 } })
at(147.208) world("ENCHANT_SPELL_COMPLETED", false, nil)
at(156.708) player:softTarget(caster, "friend")
at(157.375) caster:setPower(990)
at(159.308) caster:setPower(1000)
at(161.159) caster:castSpell(8004, { castTime = 2.5 }) caster:setPower(950)
at(161.373) caster:setPower(960)
at(161.993) player:softTarget(other, "friend")
-- At the forge: Smelting again; bronze and copper picked.
player:setSpellFocus({ "Forge" })
local page = ProfessionsFrame and ProfessionsFrame.CraftingPage
at(167.288) player:openTradeSkill(2656, "Mining")
-- Another caster soft targeted, spending mana.
local caster2 = e.spawnUnit({ name = "Crossroads Caster", reaction = "friendly", powerType = 0, power = 1000, maxPower = 1000 })
at(169.145) player:softTarget(caster2, "friend")
at(170.091) pick(2659)
at(170.693) caster2:setPower(990)
at(171.114) pick(2657)
check("copper picked", page.SchematicForm:GetRecipeInfo().recipeID == 2657)
-- Every copper bar: casts of 3 s, one after another (each next cast
-- ~0.3 s after the last one's success: the ping there).
e.setLatency(0.3)
-- The casts' timings as recorded (the server's answers, seconds): each
-- start after its click or the last success, its success after its
-- start, its item after the success, its bags' packet after the item, a
-- new slot's item data after the item (the first tin and bronze bars'):
-- four copper bars, the tin the server held, four tin bars (the last
-- interrupted), four bronze bars. The first bar's bags in the moment of
-- the next cast's start, before it (0.281 recorded for both).
player:craftTimings({
    { start = 0.335, success = 2.951, item = 0.068, bags = 0.2809 },
    { start = 0.349, success = 2.939, item = 0, bags = 0.325 },
    { start = 0.325, success = 3.030, item = 0, bags = 0.186 },
    { start = 0.299, success = 2.986, item = 0.044, bags = 0.124 },
    { start = 0.293, success = 2.906, item = 0.096, bags = 0.098, data = 0.425 },
    { start = 0.327, success = 2.932, item = 0.024, bags = 0.151 },
    { start = 0.342, success = 2.987, item = 0, bags = 0.413 },
    { start = 0.310, success = 2.940, item = 0.035, bags = 0.396 },
    { start = 0.315 },
    { start = 0.323, success = 3.001, item = 0.041, bags = 0.343, data = 1.034 },
    { start = 0.278, success = 2.967, item = 0.032, bags = 0.387 },
    { start = 0.297, success = 2.952, item = 0.038, bags = 0.346 },
    { start = 0.320, success = 2.982, item = 0.057, bags = 0.240 },
})
-- The area's updates without a zone's event (recorded, as on the taxi
-- flight; their cause unknown).
at(187.105) e.areaChanged()
at(188.714) player:setMoving(false)
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
at(219.122) player:setMoving(true) player:interruptCast()
at(219.249) player:setMoving(false)
at(219.590) press(page.CreateAllButton)
at(234.0)
check("the bronze made", C_Item.GetItemCount(2841) == 8)
at(235.277) e.pressKey("ESCAPE")
at(236.0)
