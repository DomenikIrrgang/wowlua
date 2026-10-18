-- /stdtest engine scenario talents, played with the engine's controls: the
-- level 11 shaman opens its talents (N), buys a rank of node 104753 and
-- applies it (scenarios/talents.lua). Each step at its recorded time. Not
-- yet: the talent's PLAYER_DAMAGE_DONE_MODS as it is applied; the rest of
-- the recording (a gossip at 13 s, attacked at 39 s, walking; following and
-- inspecting the druid at 347 s).
local e = __engine
-- The shaman's hidden group buffs (HIDDEN_GROUP_BUFFS_CHANGED, recorded).
e.setHiddenGroupBuffs({ 8185, 10613, 10478, 10534, 10441, 15036, 10609, 8836, 15111, 8249, 1239242, 10491, 17354, 8076, 8162, 15107, 10537, 16176, 8160, 10535, 15112, 8184, 8163, 10538, 10614, 16191, 17355, 5677, 10526, 8835, 17360, 10494, 8515, 8182, 10497, 15109, 10496, 8512, 16190, 10493, 8181, 10626, 8230, 10479, 1239243, 25362, 10495, 8227, 15108, 15110, 408521, 1325257, 10627, 10477, 17359, 16387, 8075, 1325256, 10521, 25360, 10442, 25361, 10476, 5678, 10612, 8161, 25359, 8250 })
-- Before: the node not bought yet (the probe's ranks are the end's: 1).
e.setTraitRanks(5999541, { [104753] = 0 })
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
    if not frame then
        check("clicked: no frame", false)
        return
    end
    local ok, err = pcall(e.click, frame, button)
    if not ok then check("clicked: " .. tostring(err), false) end
end
e.setLatency(0.465)
at(5.969) e.pressKey("N")
-- Walking into the town (6.333 on): its walls' chunk, resting.
at(7.078) e.newWmoChunk()
at(7.847) e.setResting(true)
check("the talents shown", PlayerSpellsFrame and PlayerSpellsFrame:IsShown()
    and PlayerSpellsFrame.TalentsFrame:IsVisible())
local talents = PlayerSpellsFrame.TalentsFrame
local node = talents:GetTalentButtonByNodeID(104753)
check("the node's button shown", node and node:IsVisible())
at(8.508) press(node)
at(10.078) press(talents.ApplyButton)
at(11.0)
check("the rank applied", C_Traits.GetNodeInfo(5999541, 104753).ranksPurchased == 1
    and not C_Traits.ConfigHasStagedChanges(5999541))
at(12.0)
