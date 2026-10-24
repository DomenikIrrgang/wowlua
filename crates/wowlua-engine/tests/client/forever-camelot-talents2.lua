-- WoW Forever 1.60.1 on Windows, a Paladin with no unspent talent
-- point: /stdtest engine talents2 (tests/ingame/src), recorded 2026-10-24.
-- Both spec groups' configs, the active config, a rank planned and taken
-- back (PurchaseRank refused: no point to spend, so nothing changed), and
-- another class's talents in the view config.
return {
    ["refund: RollbackConfig"] = "1: boolean false",
    ["view: ViewLoadout({}, \"\")"] = "1: boolean true",
    ["refund: after purchase"] = "{ID=number 105332,activeEntry={entryID=number 130026,rank=number 0},activeRank=number 0,canPurchaseRank=boolean true,canRefundRank=boolean false,conditionIDs={},currentRank=number 0,entryIDs={1=number 130026},entryIDsWithCommittedRanks={},flags=number 0,groupIDs={1=number 12745,2=number 12746,3=number 12747,4=number 12748,5=number 12749,6=number 11486},isAvailable=boolean true,isCascadeRepurchasable=boolean false,isDisplayError=boolean false,isVisible=boolean true,maxRanks=number 5,meetsEdgeRequirements=boolean true,nextEntry={entryID=number 130026,rank=number 1},posX=number 2220,posY=number 2130,ranksIncreased=number 0,ranksPurchased=number 0,totalMaxRanks=number 5,type=number 0,visibleEdges={}}",
    ["GetConfigInfo(active)"] = "1: {ID=number 73810,name=string Paladin,treeIDs={1=number 1100},type=number 4,usesSharedActionBars=boolean false}",
    ["refund: PurchaseRank"] = "1: boolean false",
    ["refund: before"] = "{ID=number 105332,activeEntry={entryID=number 130026,rank=number 0},activeRank=number 0,canPurchaseRank=boolean true,canRefundRank=boolean false,conditionIDs={},currentRank=number 0,entryIDs={1=number 130026},entryIDsWithCommittedRanks={},flags=number 0,groupIDs={1=number 12745,2=number 12746,3=number 12747,4=number 12748,5=number 12749,6=number 11486},isAvailable=boolean true,isCascadeRepurchasable=boolean false,isDisplayError=boolean false,isVisible=boolean true,maxRanks=number 5,meetsEdgeRequirements=boolean true,nextEntry={entryID=number 130026,rank=number 1},posX=number 2220,posY=number 2130,ranksIncreased=number 0,ranksPurchased=number 0,totalMaxRanks=number 5,type=number 0,visibleEdges={}}",
    ["refund: RefundRank"] = "1: boolean false",
    ["view: class, spec"] = "1 1491",
    ["refund: node"] = "105332",
    ["GetActiveSpecGroup()"] = "error: attempt to call a nil value",
    ["refund: after refund"] = "{ID=number 105332,activeEntry={entryID=number 130026,rank=number 0},activeRank=number 0,canPurchaseRank=boolean true,canRefundRank=boolean false,conditionIDs={},currentRank=number 0,entryIDs={1=number 130026},entryIDsWithCommittedRanks={},flags=number 0,groupIDs={1=number 12745,2=number 12746,3=number 12747,4=number 12748,5=number 12749,6=number 11486},isAvailable=boolean true,isCascadeRepurchasable=boolean false,isDisplayError=boolean false,isVisible=boolean true,maxRanks=number 5,meetsEdgeRequirements=boolean true,nextEntry={entryID=number 130026,rank=number 1},posX=number 2220,posY=number 2130,ranksIncreased=number 0,ranksPurchased=number 0,totalMaxRanks=number 5,type=number 0,visibleEdges={}}",
    ["view: VIEW_TRAIT_CONFIG_ID"] = "-3",
    ["GetActiveConfigID()"] = "1: number 73810",
    ["GetCombatConfigIDForSpecGroup(2)"] = "1: nil nil",
    ["view: GetConfigInfo(view) after ViewLoadout"] = "1: {ID=number -3,name=string ,treeIDs={1=number 1117},type=number 1,usesSharedActionBars=boolean false}",
    ["view: InitializeViewLoadout(spec, 60)"] = "0: ",
    ["view: GetConfigInfo(view)"] = "1: {ID=number -3,name=string ,treeIDs={1=number 1117},type=number 1,usesSharedActionBars=boolean false}",
    ["GetNumSpecGroups()"] = "1: number 1",
    ["GetCombatConfigIDForSpecGroup(1)"] = "1: number 73810",
}
