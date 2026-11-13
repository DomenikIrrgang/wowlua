//! The loot rolls' state (wowlua-engine's `lootroll` answers from it).

/// A choice on a roll.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Choice {
    Need,
    Greed,
    Pass,
}

impl Choice {
    /// `RollOnLoot`'s roll type (0 Pass, 1 Need, 2 Greed).
    pub fn of_type(t: f64) -> Option<Choice> {
        [Choice::Pass, Choice::Need, Choice::Greed]
            .into_iter()
            .find(|ch| ch.roll_type() == t)
    }

    pub fn of_name(s: &str) -> Option<Choice> {
        match s {
            "need" => Some(Choice::Need),
            "greed" => Some(Choice::Greed),
            "pass" => Some(Choice::Pass),
            _ => None,
        }
    }

    pub fn roll_type(self) -> f64 {
        match self {
            Choice::Pass => 0.0,
            Choice::Need => 1.0,
            Choice::Greed => 2.0,
        }
    }

    /// `Enum.EncounterLootDropRollState`: NeedMainSpec, Greed, Pass.
    pub fn state(self) -> f64 {
        match self {
            Choice::Need => 0.0,
            Choice::Greed => 3.0,
            Choice::Pass => 5.0,
        }
    }
}

/// A group member on a roll (the player first).
#[derive(Clone, Debug)]
pub struct Member {
    pub name: String,
    pub guid: String,
    pub class: String,
    pub is_self: bool,
    pub choice: Option<Choice>,
    /// When the choice was made, among the roll's choices.
    pub order: usize,
    /// The choice told by the server (the player's: a round trip later).
    pub told: bool,
    /// The number the test gives for the member's roll.
    pub given: Option<f64>,
    /// The number rolled at the end.
    pub roll: Option<f64>,
}

/// A roll.
#[derive(Clone, Debug)]
pub struct Roll {
    pub id: f64,
    pub item: i64,
    pub count: f64,
    /// Its time in seconds, and when it began (`GetTime`'s seconds).
    pub duration: f64,
    pub started: f64,
    pub bop: bool,
    pub can_need: bool,
    pub can_greed: bool,
    pub reason_need: f64,
    pub reason_greed: f64,
    /// The loot it is from (`lootHandle`).
    pub loot: f64,
    pub encounter: f64,
    pub key: f64,
    pub members: Vec<Member>,
    pub choices: usize,
    pub done: bool,
    pub winner: Option<usize>,
}

impl Roll {
    pub fn everyone_chosen(&self) -> bool {
        self.members.iter().all(|m| m.choice.is_some() && m.told)
    }
}

/// The rolls of the session and the loot history's encounters.
#[derive(Default)]
pub struct LootRolls {
    pub rolls: Vec<Roll>,
    pub next_id: f64,
    pub next_loot: f64,
    /// The encounters with drops: ID, name, the first drop's time.
    pub encounters: Vec<(f64, String, f64)>,
    pub seed: u32,
}
