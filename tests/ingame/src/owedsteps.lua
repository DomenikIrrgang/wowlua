-- `/stdtest engine owed` (owed.wowlua): the owed probes of
-- tests/ingame/PROBES.md that are pure code, in one run. Each step is the
-- `/run` line PROBES.md gives (the same calls, the same IDs), with `print`
-- the recorder's: its values land under the step's key ("M12.3.2": the
-- section "Owed from the M12 screenshots", item 3, step 2), so they compare
-- 1:1 with PROBES.md's "wowlua:" lines. `/dump` lines use `dump` (tables
-- flattened). The runner takes one step per frame (as lines typed one
-- after another), waits where a step says so, and records errors as text.
-- Plain Lua, so the steps read as PROBES.md writes them.
local _, ns = ...

local steps = {}
local kept = {}

--- A step: its key, its code (given print and dump), and how long to wait
--- before it (at least the next frame).
local function step(key, run, wait)
    steps[#steps + 1] = { key = key, run = run, wait = wait }
end

--- Something a step made, hidden when its item is done.
local function keep(o)
    kept[#kept + 1] = o
    return o
end

--- Hides what the steps made so far (their scripts cleared).
local function release()
    for _, o in ipairs(kept) do
        if type(o) == "table" then
            if o.UnregisterAllEvents then pcall(o.UnregisterAllEvents, o) end
            if o.SetScript and o.HasScript then
                for _, s in ipairs({ "OnUpdate", "OnEvent", "OnTextChanged", "OnSizeChanged", "OnValueChanged", "OnMinMaxChanged", "OnEditFocusGained", "OnArrowPressed" }) do
                    local ok, has = pcall(o.HasScript, o, s)
                    if ok and has then pcall(o.SetScript, o, s, nil) end
                end
            end
            if o.ClearFocus then pcall(o.ClearFocus, o) end
            if o.Hide then pcall(o.Hide, o) end
        end
    end
    kept = {}
end

--- A step hiding what the item made, a frame after its last step.
local function done()
    step(nil, function() release() end)
end

------------------------------------------------------------------------
-- Owed from the M12 screenshots
------------------------------------------------------------------------

-- 1. Load-on-demand addons' reason (decision T354). Step 2 loads
-- AtlasLootClassic_Crafting if it is installed (a load-on-demand addon:
-- nothing else changes); without it the client answers for a missing one.
step("M12.1.1", function(print)
    for i=1,C_AddOns.GetNumAddOns() do local n,_,_,l,r=C_AddOns.GetAddOnInfo(i) if C_AddOns.IsAddOnLoadOnDemand(i) then print(n,tostring(l),"["..tostring(r).."]",C_AddOns.IsAddOnLoaded(i)) end end
end)
step("M12.1.2", function(print)
    print(C_AddOns.LoadAddOn("AtlasLootClassic_Crafting"), "["..tostring(select(5,C_AddOns.GetAddOnInfo("AtlasLootClassic_Crafting"))).."]")
end)
step("M12.1.1 again", function(print)
    for i=1,C_AddOns.GetNumAddOns() do local n,_,_,l,r=C_AddOns.GetAddOnInfo(i) if C_AddOns.IsAddOnLoadOnDemand(i) then print(n,tostring(l),"["..tostring(r).."]",C_AddOns.IsAddOnLoaded(i)) end end
end)

-- 2. AceGUI's SetDesaturation.
step("M12.2.1", function(print)
    print(SetDesaturation, LibStub and LibStub("AceGUI-3.0",true) and select(2,LibStub("AceGUI-3.0")))
end)

-- 3. Which point sets an edge (decision T337).
do
    local F
    step("M12.3.1", function(print)
        F=CreateFrame("Frame",nil,UIParent) F:SetHeight(10) F:SetPoint("RIGHT",UIParent,"CENTER") F:SetPoint("LEFT",UIParent,"CENTER",-100,0) F:SetPoint("TOPLEFT",UIParent,"CENTER",-50,0)
        keep(F)
    end)
    step("M12.3.2", function(print)
        print(UIParent:GetWidth()/2-F:GetLeft(), F:GetWidth(), (F:GetPoint(1)))
    end)
    done()
end

-- 4. A button's font objects by name (decision T338).
step("M12.4.1", function(print)
    local b=CreateFrame("Button",nil,UIParent) keep(b) b:SetNormalFontObject("GameFontRed") print(b:GetNormalFontObject():GetName()) print(pcall(b.SetHighlightFontObject,b,"NoSuchFont"))
end)
done()

-- 7. SetFrameLevel and descendants (decision T343).
do
    local P, C, G
    step("M12.7.1", function(print)
        P=CreateFrame("Frame",nil,UIParent) keep(P) C=CreateFrame("Frame",nil,P) C:SetFrameLevel(P:GetFrameLevel()+5) G=CreateFrame("Frame",nil,C) print(P:GetFrameLevel(),C:GetFrameLevel(),G:GetFrameLevel())
    end)
    step("M12.7.2", function(print)
        P:SetFrameLevel(P:GetFrameLevel()+10) print(P:GetFrameLevel(),C:GetFrameLevel(),G:GetFrameLevel())
    end)
    done()
end

-- 8. Inline textures in text (decision T347; the screenshot stays manual).
do
    local S
    step("M12.8.1", function(print)
        S=UIParent:CreateFontString(nil,"OVERLAY","GameFontNormal") keep(S) S:SetPoint("CENTER") S:SetText("ab") print(S:GetStringWidth(),S:GetStringHeight())
    end)
    step("M12.8.2", function(print)
        S:SetText("a\124TInterface\\Icons\\INV_Misc_QuestionMark:0\124tb\124A:128-RedButton-Exit:20:20\124ac") print(S:GetStringWidth(),S:GetStringHeight())
    end)
    done()
end

-- 11. .png and .jpg paths of .blp files (decision T350).
step("M12.11.1", function(print)
    local t=UIParent:CreateTexture() keep(t) for _,x in ipairs({".blp",".tga",".png",".jpg",""}) do print(x,t:SetTexture("Interface\\FriendsFrame\\StatusIcon-Online"..x),t:GetTexture()) end
end)

-- 14. An unknown atlas (decision T346).
step("M12.14.1", function(print)
    local t=UIParent:CreateTexture() keep(t) t:SetColorTexture(1,0,0) print(t:SetAtlas("NoSuchAtlas"), t:GetAtlas(), t:GetTexture())
end)
done()

-- 16. Tooltip spacing (decision T342; the screenshot stays manual).
step("M12.16.1", function(print)
    GameTooltip:SetOwner(UIParent,"ANCHOR_NONE") GameTooltip:SetPoint("CENTER") GameTooltip:AddLine("Title") GameTooltip:AddDoubleLine("Left","Right")
end)
step("M12.16.2", function(print)
    GameTooltip:AddLine("A long line to see where and how a tooltip wraps its text",1,1,1,true) GameTooltip:Show() print(GameTooltip:GetSize())
end)
step("M12.16.2 a frame later", function(print)
    print(GameTooltip:GetSize()) print(GameTooltip:NumLines()) GameTooltip:Hide()
end)

-- 17. PvP ranks 5 to 14 (decision T345).
step("M12.17.1", function(print)
    for n=5,14 do print(n,C_MajorFactions.GetTotalReputationForRenownLevel(2800,n)) end
end)
step("M12.17.2", function(print)
    for n=2,6 do local r=C_MajorFactions.GetRenownRewardsForLevel(2800,n) print(n,#r,r[1] and (r[1].name or r[1].itemID)) end
end)

-- 18. The calendar's holidays, step 2 (step 1, the screenshot, stays
-- manual). A title that is not a holiday's (a player's or a guild's event)
-- is recorded as its length only. Again after asking the server for the
-- calendar (C_Calendar.OpenCalendar(), what opening it does), 2 s later.
do
    local function title(e)
        if e.calendarType == "HOLIDAY" then return e.title end
        return "<" .. #tostring(e.title) .. " characters>"
    end
    local function holidays(print)
        local m=C_Calendar.GetMonthInfo(0) print("month",m.month,m.year,m.numDays) for d=1,m.numDays do for j=1,C_Calendar.GetNumDayEvents(0,d) do local e=C_Calendar.GetDayEvent(0,d,j) print(d,title(e),e.calendarType,e.sequenceType,e.eventID) end end
    end
    step("M12.18.2", holidays)
    step(nil, function() C_Calendar.OpenCalendar() end)
    step("M12.18.2 after OpenCalendar", holidays, 2)
end

-- 19. The skill bars' fill, steps 7 and 9 (the others open the character
-- window, step 8 is a screenshot: manual).
step("M12.19.7", function(print)
    local T=UIParent:CreateTexture(nil,"OVERLAY") keep(T) T:SetAtlas("common-stat-bar-blue",true) T:SetPoint("LEFT",UIParent,"CENTER",-100,0) T:SetWidth(0) print(T:GetSize())
end)
step("M12.19.9", function(print)
    local p=CreateFrame("Frame",nil,UIParent) keep(p) p:SetPoint("CENTER") p:SetSize(100,50) local c=CreateFrame("Frame",nil,p) c:SetAllPoints() local g=CreateFrame("Frame",nil,c) g:SetAllPoints() print(g:GetWidth(),g:IsRectValid(),c:IsRectValid())
end)
done()

-- 20d. Mouse scripts turn the mouse on (decision T365), steps 1 and 2; step 3
-- also without opening the character frame (with it open: manual).
step("M12.20d.1", function(print)
    local f=CreateFrame("Frame") local a=f:IsMouseEnabled() f:SetScript("OnMouseUp",function() end) local b=f:IsMouseEnabled() f:SetScript("OnMouseUp",nil) print(a,b,f:IsMouseEnabled())
end)
step("M12.20d.2", function(print)
    local g=CreateFrame("Frame") g:SetScript("OnEnter",function() end) local h=CreateFrame("Frame") h:SetScript("OnMouseWheel",function() end) print(g:IsMouseEnabled(),h:IsMouseEnabled(),h:IsMouseWheelEnabled())
end)
step("M12.20d.3 (character frame not opened)", function(print)
    local t=CharacterFrameModeTab2 print(t and t:IsMouseEnabled(),t and t:IsMouseClickEnabled(),t and t:IsMouseMotionEnabled())
end)

-- 20e. BNGetInfo(): step 3; and how many values it gives now (the count
-- only: its second value is the account's BattleTag).
step("M12.20e.3", function(print)
    print(select("#",BNGetInfo("player")),BNConnected())
end)
step("M12.20e.1 (count only, not at login)", function(print)
    print(select("#",BNGetInfo()))
end)

------------------------------------------------------------------------
-- Owed from M13
------------------------------------------------------------------------

-- 1. A tooltip's wrap width (decision T393; the screenshots stay manual).
step("M13.1.1", function(print)
    GameTooltip:SetOwner(UIParent,"ANCHOR_CURSOR") GameTooltip:SetSpellByID(26573) GameTooltip:Show() print(GameTooltip:GetWidth())
end)
step("M13.1.1 a frame later", function(print)
    print(GameTooltip:GetWidth())
end)
step("M13.1.2", function(print)
    GameTooltip:SetOwner(UIParent,"ANCHOR_CURSOR") GameTooltip:AddLine("Short") GameTooltip:AddLine(string.rep("word ",60),1,1,1,true) GameTooltip:Show() print(GameTooltip:GetWidth())
end)
step("M13.1.2 a frame later", function(print)
    print(GameTooltip:GetWidth()) GameTooltip:Hide()
end)

-- 2. A spell's description with spell power (decision T391; meaningful on a
-- paladin with spell power gear).
step("M13.2 class", function(print)
    print(select(2,UnitClass("player")), UnitLevel("player"))
end)
step("M13.2.1", function(print)
    print(GetSpellBonusDamage(2), GetSpellBonusHealing())
end)
step("M13.2.2", function(print)
    print(C_Spell.GetSpellDescription(26573))
end)
step("M13.2.3", function(print)
    print(C_Spell.GetSpellDescription(635))
end)

-- 3. Currencies, factions and mounts the character doesn't have.
step("M13.3.1", function(_, dump) dump(C_CurrencyInfo.GetCurrencyInfo(1792)) end)
step("M13.3.2", function(_, dump) dump(C_CurrencyInfo.GetCurrencyInfo(1901)) end)
step("M13.3.3", function(_, dump) dump(C_Reputation.GetFactionDataByID(69)) end)
step("M13.3.4", function(_, dump) dump(C_Reputation.GetFactionDataByID(930)) end)
step("M13.3.5", function(_, dump) dump(C_MountJournal.GetMountInfoByID(6)) end)

------------------------------------------------------------------------
-- Owed from M14
------------------------------------------------------------------------

-- 1. When SETTINGS_PANEL_OPEN comes (decision T400). The event's line goes
-- to step 2's record, so the three lines keep their order. The panel is
-- closed after (HideUIPanel: nothing to commit, no game menu).
do
    local f
    local out = function() end
    step("M14.1 settings shown before", function(print)
        print(SettingsPanel and SettingsPanel:IsShown())
    end)
    step("M14.1.1", function(print)
        f=CreateFrame("Frame") keep(f) f:RegisterEvent("SETTINGS_PANEL_OPEN") f:SetScript("OnEvent",function(_,e,a,b) out("event",a,b,GetTime()) end)
    end)
    step("M14.1.2", function(print)
        out=print
        print("before",GetTime()) C_SettingsUtil.OpenSettingsPanel() print("after",GetTime(),SettingsPanel:IsShown())
    end)
    step("M14.1.2 a frame later", function(print)
        print(SettingsPanel:IsShown())
        if SettingsPanel:IsShown() then HideUIPanel(SettingsPanel) end
        print("closed", SettingsPanel:IsShown())
    end)
    done()
end

------------------------------------------------------------------------
-- Owed from the contributions in 0.13.5
------------------------------------------------------------------------

-- 1. GetTotemInfo beyond the first four slots (#81).
step("0.13.5.1.1", function(print)
    for s=0,9 do print(s, select("#", GetTotemInfo(s)), GetTotemInfo(s)) end
end)

-- 2. Aura options left out (#82).
step("0.13.5.2.1", function(_, dump) dump(C_AuraContainerUtil.ProcessCustomAuraButtonDurationTextOptions({})) end)
step("0.13.5.2.2", function(_, dump) dump(C_AuraContainerUtil.ProcessAuraTooltipBackdropOptions({ backdropInfo = {} })) end)

-- 4. The heal prediction calculator (decision T486).
step("0.13.5.4.1", function(print)
    local c=CreateUnitHealPredictionCalculator() print(c:GetIncomingHealOverflowPercent(), c:GetMaximumHealthMode(), c:GetHealAbsorbMode(), c:GetDamageAbsorbClampMode(), c:GetHealAbsorbClampMode(), c:GetIncomingHealClampMode())
end)
step("0.13.5.4.2", function(print)
    local c=CreateUnitHealPredictionCalculator() c:SetPredictedValues({health=60000,healthMax=100000,totalDamageAbsorbs=36000,totalHealAbsorbs=5000,totalIncomingHeals=12000,totalIncomingHealsFromHealer=8000}) print(c:GetIncomingHeals()) print(c:GetHealAbsorbs()) print(c:GetDamageAbsorbs()) print(c:GetMaximumIncomingHeals(), c:GetMaximumDamageAbsorbs(), c:GetMaximumHealAbsorbs())
end)
step("0.13.5.4.3", function(print)
    local c=CreateUnitHealPredictionCalculator() c:SetPredictedValues({health=60000,healthMax=100000,totalDamageAbsorbs=36000,totalHealAbsorbs=5000,totalIncomingHeals=12000,totalIncomingHealsFromHealer=8000}) c:SetHealAbsorbMode(Enum.UnitHealAbsorbMode.Total) print(c:GetIncomingHeals()) print(c:GetHealAbsorbs()) print(c:GetDamageAbsorbs())
end)

-- 6. A status bar on a timer (decision T487): a white bar in the middle of
-- the screen for about 8 seconds. WOWLUA_VC and WOWLUA_MC are upvalues
-- here; the waits are PROBES.md's "about" (each step's time is recorded).
do
    local WOWLUA_BAR, WOWLUA_BAR2, WOWLUA_VC, WOWLUA_MC
    step("0.13.5.6.1", function(print)
        local b=CreateFrame("StatusBar",nil,UIParent) keep(b) b:SetSize(200,20) b:SetPoint("CENTER") b:SetStatusBarTexture("Interface\\Buttons\\WHITE8X8") print(b:GetMinMaxValues()) b:SetScript("OnValueChanged",function() WOWLUA_VC=(WOWLUA_VC or 0)+1 end) b:SetScript("OnMinMaxChanged",function() WOWLUA_MC=(WOWLUA_MC or 0)+1 end) local d=C_DurationUtil.CreateDuration() d:SetTimeFromStart(GetTime(),10) b:SetTimerDuration(d) WOWLUA_BAR=b print(b:GetMinMaxValues()) print(b:GetValue())
    end)
    step("0.13.5.6.2", function(print)
        print(WOWLUA_BAR:GetMinMaxValues()) print(WOWLUA_BAR:GetValue(), WOWLUA_VC, WOWLUA_MC)
    end, 3)
    step("0.13.5.6.3", function(print)
        WOWLUA_BAR:SetValue(3) print(WOWLUA_BAR:GetValue())
    end, 0.5)
    step("0.13.5.6.4", function(print)
        print(WOWLUA_BAR:GetValue())
    end, 2)
    step("0.13.5.6.5", function(print)
        local b=CreateFrame("StatusBar",nil,UIParent) keep(b) b:SetMinMaxValues(0,200) local d=C_DurationUtil.CreateDuration() d:SetTimeFromStart(GetTime(),10) b:SetTimerDuration(d,Enum.StatusBarInterpolation.Immediate,Enum.StatusBarTimerDirection.RemainingTime) WOWLUA_BAR2=b print(b:GetMinMaxValues()) print(b:GetValue())
    end)
    step("0.13.5.6.6", function(print)
        print(WOWLUA_BAR2:GetValue()) WOWLUA_BAR2:SetMinMaxValues(0,0) print(WOWLUA_BAR2:GetValue())
    end, 3)
    done()
end

------------------------------------------------------------------------
-- Still to run: GetBindingText(key, 1) with modifiers
------------------------------------------------------------------------

step("Still.GetBindingText", function(print)
    for _,k in ipairs({"SHIFT-1","CTRL-ALT-2","META-E","SHIFT-MOUSEWHEELUP","SHIFT-NUMPAD1"}) do print(k, GetBindingText(k, 1), GetBindingText(k, true), GetBindingText(k)) end
end)
step("Still.GetBindingText keys alone", function(print)
    for _,k in ipairs({"1","2","E","MOUSEWHEELUP","NUMPAD1","SHIFT","CTRL","ALT","META"}) do print(k, GetBindingText(k, 1), GetBindingText(k)) end
end)

------------------------------------------------------------------------
-- New (2026-11-09): what decision T605 left open. SuuUI's anchors3 case G
-- (PROBES.md "Found with SuuUI" item 1) and variants, each case alone: a
-- panel P, its body hanging on its top by two points, 1 high, a Button
-- item 55 by 30 with a logging OnSizeChanged hook; place() a frame later
-- anchors the item to the body and sets the body's height. The item's
-- left and top (top from UIParent's) "now", "next" (C_Timer.After(0)) and
-- "1s"; 112,-140 is placed, nil not.
--   G, I, M  the recorded cases again (lost, placed, placed)
--   Gq   G without "now": nothing asks for a rectangle before the frame's
--        end (the end-of-frame pass alone)
--   Gh   G built hidden; placed (and asked "now") hidden; shown a frame
--        later, asked at once ("shown"), next and 1 s later
--   Gh2  as Gh without "now" and "shown": the first question the next frame
--   T    the body's OnSizeChanged asks for its own size, resizes nothing
--   W    G, the body's own size asked (body:GetSize()) right after its
--        height is set, before the item is asked for ("now")
------------------------------------------------------------------------

do
    local function rect(f)
        local l, t = f:GetLeft(), f:GetTop()
        return l and string.format('%g,%g', l, t - UIParent:GetTop()) or 'nil'
    end
    local function case(name, o)
        local log, P, body, item, fit
        local function note(label) log(label .. ' ' .. rect(item)) end
        step("T605." .. name, function(print)
            log = print
            P = CreateFrame('Frame', nil, UIParent)
            keep(P)
            if o.hidden then P:Hide() end
            body = CreateFrame('Frame', nil, P)
            body:SetPoint('TOPLEFT', 12, -40)
            body:SetSize(1, 1)
            P:SetWidth(320)
            body:SetPoint('TOPRIGHT', -12, -40)
            fit = function() local _, h = body:GetSize(); P:SetHeight(40 + h + 12) end
            if o.script == 'fit' then body:SetScript('OnSizeChanged', fit)
            elseif o.script == 'self' then body:SetScript('OnSizeChanged', function(self) local _, h = self:GetSize() log('body asks ' .. string.format('%g', h)) end) end
            P:SetHeight(40 + 1 + 12)
            item = CreateFrame('Button', nil, body)
            item:SetSize(55, 30)
            item:HookScript('OnSizeChanged', function(_, w, h) log(string.format('size %gx%g', w, h)) end)
            P:SetPoint('TOPLEFT', 100, -100)
            if not o.hidden then P:Show() end
        end, 1.2)
        step(nil, function()
            log('place')
            item:ClearAllPoints()
            item:SetPoint('TOPLEFT', body, 'TOPLEFT', 0, 0)
            body:SetHeight(30)
            if o.fitNow then fit() end
            if o.bodySize then local _, h = body:GetSize() log('body ' .. string.format('%g', h)) end
            if not o.quiet then note('now') end
            if not o.hidden then
                C_Timer.After(0, function() note('next') end)
                C_Timer.After(1, function() note('1s') end)
            end
        end)
        if o.hidden then
            step(nil, function()
                log('show')
                P:Show()
                if not o.quiet then note('shown') end
                C_Timer.After(0, function() note('next') end)
                C_Timer.After(1, function() note('1s') end)
            end)
        end
    end
    case('G', { script = 'fit' })
    case('I', {})
    case('M', { fitNow = true })
    case('Gq', { script = 'fit', quiet = true })
    case('Gh', { script = 'fit', hidden = true })
    case('Gh2', { script = 'fit', hidden = true, quiet = true })
    case('T', { script = 'self' })
    case('W', { script = 'fit', bodySize = true })
    step(nil, function() release() end, 1.2)
end

------------------------------------------------------------------------
-- Other owed lines that are pure code
------------------------------------------------------------------------

-- Widget state (decision T289), steps 1-7, 11-14 (8-10 and 15-18 need a
-- click, a vendor or a guild: manual).
do
    local A, B, C, E
    step("T289.1", function(print)
        A=CreateFrame("Frame",nil,UIParent) B=CreateFrame("Frame",nil,UIParent) keep(A) keep(B) A:SetFrameStrata("FULLSCREEN_DIALOG") B:SetFrameStrata("FULLSCREEN_DIALOG") B:SetFrameLevel(7) A:Raise() print(A:GetFrameLevel(),B:GetFrameLevel())
    end)
    step("T289.2", function(print)
        C=CreateFrame("Frame",nil,UIParent) keep(C) C:SetFrameStrata("FULLSCREEN_DIALOG") C:SetFrameLevel(20) C:Hide() A:Raise() print(A:GetFrameLevel())
    end)
    step("T289.3", function(print)
        A:Lower() print(A:GetFrameLevel(),B:GetFrameLevel())
    end)
    step("T289.4", function(print)
        E=CreateFrame("EditBox",nil,UIParent) keep(E) E:SetSize(200,20) E:SetPoint("CENTER") E:SetFontObject(ChatFontNormal) E:SetAutoFocus(false) E:SetText("hello world") E:HighlightText(0,5) E:Insert("HELLO") print(E:GetText(),E:GetCursorPosition())
    end)
    step("T289.5", function(print)
        E:SetScript("OnTextChanged",function(s,u) print("changed",u) end) E:Insert("!") print("after Insert")
    end)
    step("T289.6", function(print)
        E:SetCursorPosition(1) E:SetText("abc") print(E:GetCursorPosition())
    end)
    step("T289.7", function(print)
        E:SetText("abcdef") E:HighlightText(4,1) E:Insert("Z") print(E:GetText())
    end)
    done()
end
step("T289.11", function(print)
    local F1=CreateFont("ProbeF1") F1:SetFont("Fonts\\FRIZQT__.TTF",14,"OUTLINE") F1:SetTextColor(1,0,0) local F2=CreateFont("ProbeF2") F2:CopyFontObject(F1) F1:SetTextColor(0,1,0) print(F2:GetFont()) print(F2:GetTextColor())
end)
step("T289.12", function(print)
    local P=CreateFrame("Frame",nil,UIParent) keep(P) P:SetPoint("TOPLEFT",10,20) P:AdjustPointsOffset(5,-5) print(select(4,P:GetPoint(1))) P:ClearPointsOffset() print(select(4,P:GetPoint(1)))
end)
step("T289.13", function(print)
    local Q=CreateFrame("Button",nil,UIParent) keep(Q) Q:SetNormalTexture("Interface\\Buttons\\WHITE8X8") local T=Q:GetNormalTexture() Q:ClearNormalTexture() print(Q:GetNormalTexture(),T:IsShown(),T:GetParent()==Q)
end)
step("T289.14", function(print)
    local M=CreateFrame("PlayerModel",nil,UIParent) keep(M) M:SetModel(130737) M:ClearModel() print(M:GetModelFileID())
end)
done()

-- Targeting and history (decision T291), step 3 (the keys: manual).
step("T291.3", function(print)
    local H=CreateFrame("EditBox",nil,UIParent) keep(H) H:SetSize(200,20) H:SetPoint("CENTER") H:SetFontObject(ChatFontNormal) H:SetAutoFocus(false) H:AddHistoryLine("one") H:AddHistoryLine("two") print(H:GetHistoryLines())
end)
done()

-- Leftovers (decision T295), step 4: a system line "probe line" shows.
step("T295.4", function(print)
    SendSystemMessage("probe line") print(Sign(-3),Sign(0),Sign(2))
end)

-- Model frames' cameras, step 3.
step("Model.3", function(print)
    local m=CreateFrame("PlayerModel") keep(m) m:SetSize(100,100) m:SetUnit("player") print(m:GetCameraDistance(), m:GetCameraPosition()) print(m:GetCameraTarget())
end)

-- Textures the screenshots draw, step 1 (the experience bar shown) and
-- step 4 (SuuUI's logo; without SuuUI the client answers for a missing file).
step("Textures.1", function(print)
    local t=MainStatusTrackingBarContainer.bars and MainStatusTrackingBarContainer.bars[1] or nil print(t and t.StatusBar:GetStatusBarTexture():GetAtlas(), t and t.StatusBar:GetStatusBarTexture():GetTexture())
end)
do
    local T
    step("Textures.4.1", function(print)
        T=UIParent:CreateTexture() keep(T) print(T:SetTexture("Interface\\AddOns\\SuuUI\\media\\logo"), T:GetTexture(), T:GetTextureFileID(), T:GetTextureFilePath())
    end)
    step("Textures.4.2", function(print)
        print(GetFileIDFromPath("Interface\\AddOns\\SuuUI\\media\\logo"))
    end)
    step("Textures.4.3", function(print)
        T:SetAtlas("128-RedButton-Exit") print(T:GetTexture(), T:GetTextureFileID(), T:GetTextureFilePath())
    end)
    step("Textures.4.4", function(print)
        T:SetColorTexture(1, 0, 0) print(T:GetTexture(), T:GetTextureFileID(), T:GetTextureFilePath())
    end)
    step("Textures.4.5", function(print)
        print(GetFileIDFromPath("Interface\\AddOns\\NoSuchAddon\\x"), GetFileIDFromPath("Interface\\Buttons\\WHITE8X8"))
    end)
end

-- Unanchored textures and slider thumbs, step 1 (its Lua half).
step("Unanchored.1", function(print)
    local f=CreateFrame("Frame",nil,UIParent) keep(f) f:SetSize(200,100) f:SetPoint("CENTER") local t=f:CreateTexture() t:SetSize(40,20) print(t:GetNumPoints(), t:GetSize())
end)
done()

-- The Camelot update (decision T553), steps 5 and 6 (the latter's
-- profession window: manual).
step("Camelot.5", function(print)
    for _,c in ipairs(C_UIColor.GetColors()) do if c.baseTag=="COMPACT_UNIT_FRAME_FRIENDLY_HEALTH_COLOR" then print(c.color:GetRGBA()) end end
end)
step("Camelot.6", function(print)
    local b=CreateFrame("Button") keep(b) b:SetNormalTexture("Interface\\Buttons\\WHITE8X8") local t=b:GetNormalTexture() b:ClearNormalTexture() print(b:GetNormalTexture()==t, t:GetTexture(), t:IsShown())
end)
done()

-- Unanchored font strings (decision T602), step 1.
step("T602.1", function(print)
    local r=MapQuestInfoRewardsFrame print(r.QuestSessionBonusReward:GetNumPoints(), r.PlayerTitleText:GetNumPoints())
end)

-- Names over units (decision T534), step 1; how far the world is drawn
-- (decision T525), step 1.
step("Names.1", function(print)
    for _,n in ipairs({"UnitNameNPC","UnitNameInteractiveNPC","UnitNameHostleNPC","UnitNameFriendlySpecialNPCName","UnitNameNonCombatCreatureName","nameplateShowAll","nameplateShowEnemies","nameplateShowFriendlyNpcs","nameplateMaxDistance"}) do print(n, GetCVar(n)) end
end)
step("WorldDistance.1", function(print)
    for _,n in ipairs({"farclip","terrainLodDist","horizonStart","lodObjectCullSize","lodObjectMinSize","wmoLodDist"}) do print(n, GetCVar(n)) end
end)

-- Found with SuuUI: 2 (SuuUI's font: without SuuUI the font fails to set,
-- recorded), 7 (the pet journal), 9 (the bags' families).
step("SuuUI.2", function(print)
    local f=UIParent:CreateFontString(nil,"OVERLAY") keep(f) print("SetFont", f:SetFont("Interface\\AddOns\\SuuUI\\Media\\Fonts\\Figtree-Medium.ttf",11,"")) f:SetWidth(160) f:SetText("Throw a handful into the campfire, transforming it into a violet flame.") print(f:GetStringHeight(), f:GetNumLines(), f:GetUnboundedStringWidth())
end)
done()
step("SuuUI.7.1", function(print)
    local n,o=C_PetJournal.GetNumPets() print(n,o,C_PetJournal.GetNumPetsNeedingFanfare())
end)
step("SuuUI.7.2", function(print)
    print(C_PetJournal.GetPetInfoByIndex(1))
end)
step("SuuUI.7.3", function(print)
    local _,s=C_PetJournal.GetPetInfoByIndex(1) print(C_PetJournal.GetPetModelSceneInfoBySpeciesID(s), select(12, C_PetJournal.GetPetInfoBySpeciesID(s)))
end)
step("SuuUI.9.1", function(print)
    for b=1,5 do print(b, C_Container.GetContainerNumFreeSlots(b)) end
end)
step("SuuUI.9.2", function(print)
    for b=1,5 do local s=C_Container.ContainerIDToInventoryID(b) local id=GetInventoryItemID("player",s) print(b, id, id and C_Item.GetItemFamily(id)) end
end)

ns.__wowlua.define("owedsteps", { steps = steps, release = release })
