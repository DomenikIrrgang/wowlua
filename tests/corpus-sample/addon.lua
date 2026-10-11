-- MIT License: sample code for the wowlua test suite.
local addonName, ns = ...
local L = setmetatable({}, { __index = function(t, k) return k end })

local Tracker = {}
Tracker.__index = Tracker

function Tracker.new(db)
    local self = setmetatable({}, Tracker)
    self.db = db or {} -- saved data
    self.kills, self.deaths = 0, 0
    return self
end

function Tracker:record(n, ...)
    n = n or 1
    local extra = select("#", ...)
    self.kills = self.kills + n + extra
    if self.kills > 100 then self.kills = 100
    elseif self.kills < 0 then
        self.kills = 0
    end
    return self.kills
end

local frame = CreateFrame("Frame")
frame:RegisterEvent("PLAYER_LOGIN"); frame:RegisterEvent("PLAYER_LOGOUT")
frame:SetScript("OnEvent", function(self, event, ...)
    if event == "PLAYER_LOGIN" then
        ns.tracker = Tracker.new(_G[addonName .. "DB"])
        print(("%s loaded: %d kills"):format(addonName, ns.tracker.kills))
    else -- logout
        _G[addonName .. "DB"] = ns.tracker and ns.tracker.db
    end
end)

SLASH_SAMPLE1 = "/sample"
SlashCmdList["SAMPLE"] = function(msg)
    local cmd, rest = msg:match("^(%S*)%s*(.-)$")
    if cmd == "reset" then ns.tracker.kills = 0 return end
    for word in rest:gmatch("%S+") do
        repeat
            if word == "" then break end
            print(L[word])
        until true
    end
end
