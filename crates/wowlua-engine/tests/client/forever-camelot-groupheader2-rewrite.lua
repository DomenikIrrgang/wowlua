-- WoW Forever 1.60.1 (71037): /stdtest engine groupheader rewrite (round 2), as
-- self with a secure update of the same unit between (cargo xtask ingest).
return {
    ["001"] = "client 1.60.1 (71037) variant rewrite",
    ["002"] = "shown a=player secure b=player secure refusals=0",
    ["003"] = "after rewrite a=player b=player refusals=0",
    ["004"] = "in fight lockdown=true groupFilter=2 a=player b=player button1 shown=true refusals=5 ADDON_ACTION_BLOCKED StdTest StdTestProbeHeaderBUnitButton1:Hide() x1; ADDON_ACTION_BLOCKED StdTest StdTestProbeHeaderBUnitButton1:ClearAllPoints() x1; ADDON_ACTION_BLOCKED StdTest StdTestProbeHeaderBUnitButton1:SetAttribute() x1; ADDON_ACTION_BLOCKED StdTest StdTestProbeHeaderB:SetWidth() x1; ADDON_ACTION_BLOCKED StdTest StdTestProbeHeaderB:SetHeight() x1",
    ["005"] = "fight over refusals=5 ADDON_ACTION_BLOCKED StdTest StdTestProbeHeaderBUnitButton1:Hide() x1; ADDON_ACTION_BLOCKED StdTest StdTestProbeHeaderBUnitButton1:ClearAllPoints() x1; ADDON_ACTION_BLOCKED StdTest StdTestProbeHeaderBUnitButton1:SetAttribute() x1; ADDON_ACTION_BLOCKED StdTest StdTestProbeHeaderB:SetWidth() x1; ADDON_ACTION_BLOCKED StdTest StdTestProbeHeaderB:SetHeight() x1",
    ["006"] = "recorded",
}
