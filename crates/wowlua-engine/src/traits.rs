//! Trait configs: the class talents' and the legacy system's trees, as the
//! client answered a character's (`/stdtest engine traits`,
//! `data/camelot-traits.txt` by class) and as their ranks change: a rank
//! bought is staged at once, applied (`CommitConfig`) on the server's word.
//!
//! The trees' structure is the client's `Trait` tables' where the VM has
//! them (decision T435: nodes, entries, definitions, groups, edges,
//! conditions, costs), else the recording's; the character's configs and
//! currencies' maxima are the recording's; the ranks are the engine's (the recording's unless the test sets them,
//! `e.setTraitRanks(config, { [node] = rank })`), and every answer that
//! follows from them is worked out: a node's ranks and next entry, whether
//! it is available, meets its edges, can be bought; a tree's currency; a
//! condition's points still to spend.
//!
//! Recorded (the legacy druid, the talents shaman): a staged rank counts in
//! `activeRank`, `currentRank`, `ranksPurchased` at once;
//! `entryIDsWithCommittedRanks` holds what was committed before the
//! session, a commit's ranks not even at its answer; a condition's
//! `spentAmountRequired` is what is still to spend (met: 0, without its
//! tooltip); buying fires `TRAIT_NODE_CHANGED` for the node and its edges'
//! targets, then `TRAIT_TREE_CURRENCY_INFO_UPDATED` for each of the
//! config's trees paying in its currency, then (when it unlocked others)
//! the unlocked nodes and its edges' targets again; applying: a round
//! trip, `SPELLS_CHANGED`, then the committed nodes, every tree's currency,
//! `TRAIT_CONFIG_UPDATED`, `PLAYER_TALENT_UPDATE` (a talent changing the
//! damage done: `PLAYER_DAMAGE_DONE_MODS` before the nodes,
//! `DAMAGE_DONE_MODS`).
//!
//! Unverified: an edge is met by its source at its last rank (the classic
//! rule; recorded only met at 5 of 5); the unlocked nodes' order (the
//! recording's 110303, 110305, 110304: here those without edges first); a met condition's
//! points (the probe's spent points: at most); refunds, rollbacks,
//! selection nodes; the second stage of a commit (0.5 s after the first in
//! the legacy recording, 0.07 s in the talents one).

use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, OnceLock};

use wowlua_vm::stdlib::Ret;
use wowlua_vm::{NativeCall, NativeFn, Value, Vm};

use crate::clientdata::{TraitNode, TraitTables};
use crate::units::PLAYER;
use crate::{engine, engine_mut, widgets};

/// A commit's second stage after its first, by the config's type: the
/// legacy trees' (`Generic`, 0.499 s recorded), the class talents' (0.068 s
/// recorded); unverified: why they differ.
const COMMIT_AFTER_GENERIC: f64 = 0.499;
const COMMIT_AFTER: f64 = 0.068;
/// `Enum.TraitConfigType.Generic`.
const GENERIC: f64 = 3.0;
/// The talents' spells that change the player's damage done, applied:
/// `PLAYER_DAMAGE_DONE_MODS("player")` as many times, before the nodes'
/// changes (recorded: Thundering Strikes, 1% critical strike chance with
/// all spells and attacks, four at its first rank; the legacy nodes' none).
/// Unverified: at its other ranks, other talents' (their spells' effects
/// not known to the engine).
const DAMAGE_DONE_MODS: &[(i64, usize)] = &[(16255, 4)];
/// Each class's talent tree, in the client's order, all paying in the
/// class talents' currency (recorded: a shaman's rank bought fired
/// `TRAIT_TREE_CURRENCY_INFO_UPDATED` for these, each answering it).
const CLASS_TREES: &[i64] = &[1058, 1089, 1082, 1091, 1100, 1111, 1112, 1114, 1116, 1117];
const CLASS_CURRENCY: i64 = 3820;
/// The group buffs again after a commit (recorded).
const BUFFS_AFTER: f64 = 0.007;
/// `Enum.TraitNodeFlag.NeverPurchasable`.
const NEVER_PURCHASABLE: i64 = 2;
/// `Enum.TraitNodeType.Tiered`.
const TIERED: f64 = 1.0;
/// `Enum.TraitConditionType.Available`.
const AVAILABLE: f64 = 0.0;
/// `Enum.TraitTreeFlag.CannotRefund`.
const CANNOT_REFUND: u32 = 1;
/// `Enum.TraitCondFlag`: a gate, always met, sufficient.
const IS_GATE: u32 = 1;
const IS_ALWAYS_MET: u32 = 2;
const IS_SUFFICIENT: u32 = 4;
/// `Enum.TraitEdgeType`: one source suffices, every source is needed.
const SUFFICIENT: f64 = 2.0;
const REQUIRED: f64 = 3.0;

/// A recorded value (`string x`, `number 1`, `boolean true`, `nil nil`,
/// `{k=v,…}`).
#[derive(Clone, Debug, PartialEq)]
pub enum Rv {
    Nil,
    Bool(bool),
    Num(f64),
    Str(String),
    Table(Vec<(String, Rv)>),
}

impl Rv {
    fn parse(text: &str) -> Rv {
        let text = text.trim_start();
        if let Some(inner) = text.strip_prefix('{').and_then(|t| t.strip_suffix('}')) {
            let mut fields = Vec::new();
            for entry in crate::recorded::split_top(inner, ',') {
                if let Some((k, v)) = entry.split_once('=') {
                    fields.push((k.to_string(), Rv::parse(v)));
                }
            }
            return Rv::Table(fields);
        }
        match text.split_once(' ') {
            Some(("string", s)) => Rv::Str(s.to_string()),
            Some(("number", n)) => n.parse().map_or(Rv::Nil, Rv::Num),
            Some(("boolean", b)) => Rv::Bool(b == "true"),
            _ => Rv::Nil,
        }
    }

    /// An answer's values (`n: v1, v2`).
    pub(crate) fn answer(text: &str) -> Vec<Rv> {
        let Some((count, rest)) = text.split_once(": ") else {
            return Vec::new();
        };
        if count == "0" {
            return Vec::new();
        }
        let starts = crate::recorded::regex_starts(rest);
        (0..starts.len())
            .map(|i| {
                let end = starts.get(i + 1).map_or(rest.len(), |&e| e - 2);
                Rv::parse(&rest[starts[i]..end])
            })
            .collect()
    }

    pub fn get(&self, key: &str) -> Option<&Rv> {
        match self {
            Rv::Table(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    fn num(&self, key: &str) -> Option<f64> {
        match self.get(key) {
            Some(Rv::Num(n)) => Some(*n),
            _ => None,
        }
    }

    fn flag(&self, key: &str) -> bool {
        matches!(self.get(key), Some(Rv::Bool(true)))
    }

    /// A list's values in order (`{1=…,2=…}`, keys as numbers).
    fn list(&self) -> Vec<&Rv> {
        let Rv::Table(fields) = self else {
            return Vec::new();
        };
        let mut items: Vec<(f64, &Rv)> = fields
            .iter()
            .filter_map(|(k, v)| k.parse::<f64>().ok().map(|i| (i, v)))
            .collect();
        items.sort_by(|a, b| a.0.total_cmp(&b.0));
        items.into_iter().map(|(_, v)| v).collect()
    }

    fn nums(&self, key: &str) -> Vec<i64> {
        self.get(key)
            .map(|l| {
                l.list()
                    .into_iter()
                    .filter_map(|v| match v {
                        Rv::Num(n) => Some(*n as i64),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    pub(crate) fn set(&mut self, key: &str, value: Rv) {
        if let Rv::Table(fields) = self {
            match fields.iter_mut().find(|(k, _)| k == key) {
                Some(slot) => slot.1 = value,
                None => {
                    fields.push((key.to_string(), value));
                    fields.sort_by(|a, b| a.0.cmp(&b.0));
                }
            }
        }
    }

    fn remove(&mut self, key: &str) {
        if let Rv::Table(fields) = self {
            fields.retain(|(k, _)| k != key);
        }
    }

    fn list_of(items: Vec<Rv>) -> Rv {
        Rv::Table(
            items
                .into_iter()
                .enumerate()
                .map(|(i, v)| ((i + 1).to_string(), v))
                .collect(),
        )
    }

    pub fn to_value(&self, vm: &mut Vm) -> Value {
        match self {
            Rv::Nil => Value::Nil,
            Rv::Bool(b) => Value::Bool(*b),
            Rv::Num(n) => Value::Num(*n),
            Rv::Str(s) => vm.str(s),
            Rv::Table(fields) => {
                let t = vm.new_table();
                for (k, v) in fields {
                    let key = match k.parse::<f64>() {
                        Ok(n) => Value::Num(n),
                        Err(_) => vm.str(k),
                    };
                    let value = v.to_value(vm);
                    let _ = vm.heap.table_mut(t).set(key, value);
                }
                Value::Table(t)
            }
        }
    }
}

/// The recorded answers: (class, call) → answer.
fn data() -> &'static HashMap<(&'static str, &'static str), &'static str> {
    static DATA: OnceLock<HashMap<(&'static str, &'static str), &'static str>> = OnceLock::new();
    DATA.get_or_init(|| {
        include_str!("../data/camelot-traits.txt")
            .lines()
            .filter(|l| !l.starts_with('#'))
            .filter_map(|l| {
                let (class, rest) = l.split_once('\t')?;
                let (call, answer) = rest.split_once('\t')?;
                let (call, answer) = (
                    crate::recorded::unescape(call),
                    crate::recorded::unescape(answer),
                );
                Some(((class, call), answer))
            })
            .collect()
    })
}

/// The ranks the engine keeps: by (config, node).
#[derive(Default)]
pub struct Traits {
    /// Committed ranks set by the test or a commit (else the recording's).
    pub committed: BTreeMap<(i64, i64), i64>,
    /// Ranks bought and not applied yet.
    pub staged: BTreeMap<(i64, i64), i64>,
    /// The ranks committed when the session began (set by the test).
    pub start: BTreeMap<(i64, i64), i64>,
    /// Renown by major faction (`e.setRenown`).
    pub renown: BTreeMap<i64, f64>,
    /// The recorded character playing (`e.setTraitCharacter`).
    pub character: Option<String>,
}

/// Any class's answer to a call (data by call).
/// A recorded answer's values, parsed once: the answers are the build's.
fn parsed(answer: &'static str) -> &'static [Rv] {
    type Parsed = HashMap<(usize, usize), &'static [Rv]>;
    static PARSED: OnceLock<std::sync::Mutex<Parsed>> = OnceLock::new();
    let mut parsed = PARSED
        .get_or_init(Default::default)
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    parsed
        .entry((answer.as_ptr() as usize, answer.len()))
        .or_insert_with(|| Box::leak(Rv::answer(answer).into_boxed_slice()))
}

pub(crate) fn owner_answer(call: &str) -> Option<&'static &'static str> {
    static BY_CALL: OnceLock<HashMap<&'static str, &'static str>> = OnceLock::new();
    BY_CALL
        .get_or_init(|| data().iter().map(|((_, c), a)| (*c, *a)).collect())
        .get(call)
}

/// A character's recorded trait data and the engine's ranks.
pub struct Model<'a> {
    /// The recorded character (`CLASS-configID`).
    class: String,
    state: &'a Traits,
    /// Every rank 0 (a node asked through a config without it).
    bare: bool,
    /// The class talent points the player has earned, for the player's own
    /// configs (`stats::talent_points`), in place of the recorded ones.
    points: Option<i64>,
    /// The client's trees, where its tables are at hand (decision T435):
    /// their structure in place of the recording's.
    client: Option<Arc<TraitTables>>,
}

/// An ID as the client's tables key it.
fn uid(n: i64) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// A table's fields in the order the client lists them (by key).
fn fields(mut f: Vec<(&str, Rv)>) -> Rv {
    f.sort_by(|a, b| a.0.cmp(b.0));
    Rv::Table(f.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
}

fn num(n: impl Into<f64>) -> Rv {
    Rv::Num(n.into())
}

fn nums(list: &[u32]) -> Rv {
    Rv::list_of(list.iter().map(|&n| num(n)).collect())
}

/// Costs as `GetNodeCost` lists them.
fn costs(list: &[(u32, u32)]) -> Rv {
    Rv::list_of(
        list.iter()
            .map(|&(currency, amount)| fields(vec![("ID", num(currency)), ("amount", num(amount))]))
            .collect(),
    )
}

/// A call's number arguments (`C_Traits.GetEntryInfo(5999541, 129384)`).
fn call_ints(call: &str) -> Vec<i64> {
    call.split_once('(')
        .map(|(_, rest)| rest.trim_end_matches(')'))
        .unwrap_or_default()
        .split(", ")
        .filter_map(|a| a.parse().ok())
        .collect()
}

impl<'a> Model<'a> {
    pub fn new(class: &str, state: &'a Traits) -> Model<'a> {
        Model {
            class: class.to_uppercase(),
            state,
            bare: false,
            points: None,
            client: None,
        }
    }

    /// The model of a recorded character in a VM: its ranks, and the
    /// client's trees where the VM has them.
    fn of(vm: &'a Vm, class: &str) -> Model<'a> {
        Model::new(class, &engine(vm).world.traits).with_client(crate::clientdata::traits(vm))
    }

    /// The model with the client's trees (`None`: the recording's).
    pub fn with_client(mut self, client: Option<Arc<TraitTables>>) -> Model<'a> {
        self.client = client;
        self
    }

    /// The client's node.
    fn shape(&self, node: i64) -> Option<&TraitNode> {
        self.client.as_ref()?.nodes.get(&uid(node))
    }

    /// A node's structure: the client's (nothing bought yet: its ranks are
    /// the engine's, `node`), else the recording's.
    fn node_info(&self, config: i64, node: i64) -> Option<Rv> {
        let Some(client) = &self.client else {
            return self.recorded_node(config, node);
        };
        let n = client.nodes.get(&uid(node))?;
        let max = self.max_ranks(config, node);
        let first = n.entries.first().copied();
        let entry =
            |rank: u32| first.map(|e| fields(vec![("entryID", num(e)), ("rank", num(rank))]));
        let mut f = vec![
            ("ID", num(uid(node))),
            ("activeRank", num(0)),
            ("canPurchaseRank", Rv::Bool(false)),
            ("canRefundRank", Rv::Bool(false)),
            ("conditionIDs", nums(&n.conditions)),
            ("currentRank", num(0)),
            ("entryIDs", nums(&n.entries)),
            ("entryIDsWithCommittedRanks", Rv::list_of(Vec::new())),
            ("flags", num(n.flags)),
            ("groupIDs", nums(&n.groups)),
            ("isAvailable", Rv::Bool(false)),
            ("isCascadeRepurchasable", Rv::Bool(false)),
            ("isDisplayError", Rv::Bool(false)),
            ("isVisible", Rv::Bool(true)),
            ("maxRanks", num(max as f64)),
            ("meetsEdgeRequirements", Rv::Bool(false)),
            ("posX", num(n.x)),
            ("posY", num(n.y)),
            ("ranksIncreased", num(0)),
            ("ranksPurchased", num(0)),
            ("totalMaxRanks", num(max as f64)),
            ("type", num(n.kind)),
            (
                "visibleEdges",
                Rv::list_of(
                    n.edges
                        .iter()
                        .map(|e| {
                            fields(vec![
                                ("isActive", Rv::Bool(false)),
                                ("targetNode", num(e.target)),
                                ("type", num(e.kind)),
                                ("visualStyle", num(e.style)),
                            ])
                        })
                        .collect(),
                ),
            ),
        ];
        if let Some(e) = entry(0) {
            f.push(("activeEntry", e));
        }
        if let Some(e) = entry(1) {
            f.push(("nextEntry", e));
        }
        // A tiered node's ranks increased by entry (recorded: the legacy
        // trees' one tiered node, each of its entries 0).
        if f64::from(n.kind) == TIERED {
            f.push((
                "entryIDToRanksIncreased",
                Rv::Table(n.entries.iter().map(|e| (e.to_string(), num(0))).collect()),
            ));
        }
        if n.sub_tree != 0 {
            f.push(("subTreeID", num(n.sub_tree)));
        }
        Some(fields(f))
    }

    /// The answers the client's tables give (decision T435): a tree's
    /// nodes, an entry, a definition, a node's cost. `None`: not one of
    /// those, or no client; `Some(None)`: one the client lacks.
    fn client_answer(&self, name: &str, call: &str) -> Option<Option<Rv>> {
        let client = self.client.as_ref()?;
        let args = call_ints(call);
        let arg = |i: usize| args.get(i).copied().map(uid);
        Some(match name {
            "C_Traits.GetTreeNodes" => Some(Rv::list_of(
                self.tree_nodes(args.first().copied()?)
                    .into_iter()
                    .map(|n| num(n as f64))
                    .collect(),
            )),
            "C_Traits.GetNodeCost" => Some(costs(&self.cost_list(args.get(1).copied()?))),
            "C_Traits.GetEntryInfo" => client.entries.get(&arg(1)?).map(|e| {
                let config = args[0];
                let available = e
                    .conditions
                    .iter()
                    .all(|c| self.condition_met(config, *c as i64));
                let mut f = vec![
                    ("conditionIDs", nums(&e.conditions)),
                    ("definitionID", num(e.definition)),
                    ("entryCost", costs(&e.costs)),
                    ("isAvailable", Rv::Bool(available)),
                    ("isDisplayError", Rv::Bool(false)),
                    ("maxRanks", num(e.max_ranks)),
                    ("type", num(e.kind)),
                ];
                if e.sub_tree != 0 {
                    f.push(("subTreeID", num(e.sub_tree)));
                }
                fields(f)
            }),
            "C_Traits.GetDefinitionInfo" => client.definitions.get(&arg(0)?).map(|d| {
                let mut f = Vec::new();
                for (k, v) in [
                    ("overrideName", &d.override_name),
                    ("overrideSubtext", &d.override_subtext),
                    ("overrideDescription", &d.override_description),
                ] {
                    if !v.is_empty() {
                        f.push((k, Rv::Str(v.clone())));
                    }
                }
                for (k, v) in [
                    ("spellID", d.spell),
                    ("overrideIcon", d.override_icon),
                    ("overriddenSpellID", d.overridden_spell),
                    ("visibleSpellID", d.visible_spell),
                ] {
                    if v != 0 {
                        f.push((k, num(v)));
                    }
                }
                fields(f)
            }),
            _ => return None,
        })
    }

    /// The model with the player's earned class talent points.
    fn with_points(mut self, points: i64) -> Model<'a> {
        self.points = Some(points);
        self
    }

    /// Whether the class's configs were recorded.
    fn known(&self) -> bool {
        static CLASSES: OnceLock<std::collections::HashSet<&'static str>> = OnceLock::new();
        CLASSES
            .get_or_init(|| data().keys().map(|(c, _)| *c).collect())
            .contains(self.class.as_str())
    }

    /// The class's recorded answer, else the class's whose it is (each
    /// node, entry and spell kept from the class owning its tree; an
    /// answer every class gave alike kept once).
    fn recorded(&self, call: &str) -> Option<&'static [Rv]> {
        data()
            .get(&(self.class.as_str(), call))
            .or_else(|| owner_answer(call))
            .map(|a| parsed(a))
    }

    fn first(&self, call: &str) -> Option<Rv> {
        self.recorded(call)?.first().cloned()
    }

    fn recorded_node(&self, config: i64, node: i64) -> Option<Rv> {
        self.first(&format!("C_Traits.GetNodeInfo({config}, {node})"))
    }

    pub fn trees(&self, config: i64) -> Vec<i64> {
        self.first(&format!("C_Traits.GetConfigInfo({config})"))
            .map(|c| c.nums("treeIDs"))
            .unwrap_or_default()
    }

    fn tree_nodes(&self, tree: i64) -> Vec<i64> {
        if let Some(client) = &self.client {
            return client
                .trees
                .get(&uid(tree))
                .map(|t| t.nodes.iter().map(|&n| i64::from(n)).collect())
                .unwrap_or_default();
        }
        self.first(&format!("C_Traits.GetTreeNodes({tree})"))
            .map(|l| {
                l.list()
                    .into_iter()
                    .filter_map(|v| match v {
                        Rv::Num(n) => Some(*n as i64),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default()
    }

    /// The config's nodes, tree by tree, each with its tree.
    fn nodes(&self, config: i64) -> Vec<(i64, i64)> {
        self.trees(config)
            .into_iter()
            .flat_map(|t| self.tree_nodes(t).into_iter().map(move |n| (t, n)))
            .collect()
    }

    fn tree_of(&self, config: i64, node: i64) -> Option<i64> {
        self.nodes(config)
            .into_iter()
            .find(|(_, n)| *n == node)
            .map(|(t, _)| t)
    }

    fn recorded_rank(&self, config: i64, node: i64) -> i64 {
        self.recorded_node(config, node)
            .and_then(|n| n.num("ranksPurchased"))
            .unwrap_or(0.0) as i64
    }

    fn committed(&self, config: i64, node: i64) -> i64 {
        if self.bare {
            return 0;
        }
        self.state
            .committed
            .get(&(config, node))
            .copied()
            .unwrap_or_else(|| self.recorded_rank(config, node))
    }

    /// The ranks committed as the session began.
    fn start_rank(&self, config: i64, node: i64) -> i64 {
        if self.bare {
            return 0;
        }
        self.state
            .start
            .get(&(config, node))
            .copied()
            .unwrap_or_else(|| self.recorded_rank(config, node))
    }

    fn staged(&self, config: i64, node: i64) -> i64 {
        if self.bare {
            return 0;
        }
        self.state.staged.get(&(config, node)).copied().unwrap_or(0)
    }

    fn rank(&self, config: i64, node: i64, staged: bool) -> i64 {
        self.committed(config, node) + if staged { self.staged(config, node) } else { 0 }
    }

    /// The spells of a node's entries (their definitions').
    fn node_spells(&self, config: i64, node: i64) -> Vec<i64> {
        if let Some(client) = &self.client {
            return self
                .shape(node)
                .map(|n| {
                    n.entries
                        .iter()
                        .filter_map(|e| client.entries.get(e))
                        .filter_map(|e| client.definitions.get(&e.definition))
                        .map(|d| i64::from(d.spell))
                        .collect()
                })
                .unwrap_or_default();
        }
        let entries = self
            .recorded_node(config, node)
            .map(|n| n.nums("entryIDs"))
            .unwrap_or_default();
        entries
            .into_iter()
            .filter_map(|e| {
                let def = self
                    .first(&format!("C_Traits.GetEntryInfo({config}, {e})"))?
                    .num("definitionID")? as i64;
                Some(
                    self.first(&format!("C_Traits.GetDefinitionInfo({def})"))?
                        .num("spellID")? as i64,
                )
            })
            .collect()
    }

    fn entry_max_ranks(&self, config: i64, entry: i64) -> i64 {
        if let Some(client) = &self.client {
            return client
                .entries
                .get(&uid(entry))
                .map_or(1, |e| i64::from(e.max_ranks));
        }
        self.first(&format!("C_Traits.GetEntryInfo({config}, {entry})"))
            .and_then(|e| e.num("maxRanks"))
            .unwrap_or(1.0) as i64
    }

    fn max_ranks(&self, config: i64, node: i64) -> i64 {
        // A tiered node's: its entries' together, else its first entry's.
        if let Some(n) = self.shape(node) {
            let ranks = n
                .entries
                .iter()
                .map(|&e| self.entry_max_ranks(config, i64::from(e)));
            return if f64::from(n.kind) == TIERED {
                ranks.sum()
            } else {
                ranks.take(1).sum::<i64>().max(1)
            };
        }
        self.recorded_node(config, node)
            .and_then(|n| n.num("maxRanks"))
            .unwrap_or(1.0) as i64
    }

    /// A node's cost in the client's tables: its own (and its groups'),
    /// else its tree's (recorded: every camelot node its tree's; unverified:
    /// a node with costs of its own besides).
    fn cost_list(&self, node: i64) -> Vec<(u32, u32)> {
        let Some(n) = self.shape(node) else {
            return Vec::new();
        };
        if !n.costs.is_empty() {
            return n.costs.clone();
        }
        self.client
            .as_ref()
            .and_then(|c| c.trees.get(&n.tree))
            .map(|t| t.costs.clone())
            .unwrap_or_default()
    }

    /// A node's cost: (currency, amount) a rank.
    fn cost(&self, config: i64, node: i64) -> Vec<(i64, i64)> {
        if self.client.is_some() {
            return self
                .cost_list(node)
                .into_iter()
                .map(|(c, a)| (i64::from(c), i64::from(a)))
                .collect();
        }
        self.first(&format!("C_Traits.GetNodeCost({config}, {node})"))
            .map(|l| {
                l.list()
                    .into_iter()
                    .filter_map(|c| Some((c.num("ID")? as i64, c.num("amount")? as i64)))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Points of a currency spent in the config (or one tree of it).
    fn spent(
        &self,
        config: i64,
        currency: i64,
        tree: Option<i64>,
        staged: bool,
        recorded: bool,
    ) -> i64 {
        self.spent_where(config, currency, staged, recorded, |t, _| {
            tree.is_none_or(|x| x == t)
        })
    }

    /// Points of a currency spent in the config's nodes that `within`
    /// (tree, node) holds.
    fn spent_where(
        &self,
        config: i64,
        currency: i64,
        staged: bool,
        recorded: bool,
        within: impl Fn(i64, i64) -> bool,
    ) -> i64 {
        self.nodes(config)
            .into_iter()
            .filter(|(t, n)| within(*t, *n))
            .map(|(_, n)| {
                let rank = if recorded {
                    self.recorded_rank(config, n)
                } else {
                    self.rank(config, n, staged)
                };
                let per: i64 = self
                    .cost(config, n)
                    .into_iter()
                    .filter(|(c, _)| *c == currency)
                    .map(|(_, a)| a)
                    .sum();
                rank * per
            })
            .sum()
    }

    /// `C_Traits.GetTreeCurrencyInfo(config, tree, excludeStagedChanges)`.
    pub fn tree_currency(&self, config: i64, tree: i64, exclude_staged: bool) -> Option<Rv> {
        // Another class's tree: the config's class currency (recorded).
        let own = |t: i64| {
            self.first(&format!(
                "C_Traits.GetTreeCurrencyInfo({config}, {t}, false)"
            ))
        };
        let list = own(tree).or_else(|| {
            if !CLASS_TREES.contains(&tree) {
                return None;
            }
            self.trees(config).into_iter().find_map(|t| {
                own(t).filter(|l| {
                    l.list()
                        .iter()
                        .any(|c| c.num("traitCurrencyID") == Some(CLASS_CURRENCY as f64))
                })
            })
        })?;
        let items = list
            .list()
            .into_iter()
            .map(|c| {
                let mut c = c.clone();
                if let Some(currency) = c.num("traitCurrencyID") {
                    let currency = currency as i64;
                    // The class talents' points: the player's earned
                    // ones where given, else the recording's.
                    let mut max = c.num("maxQuantity").unwrap_or(0.0) as i64;
                    if currency == CLASS_CURRENCY
                        && let Some(points) = self.points
                    {
                        max = points;
                        c.set("maxQuantity", Rv::Num(points as f64));
                    }
                    let spent = self.spent(config, currency, None, !exclude_staged, false);
                    let in_tree = self.spent(config, currency, Some(tree), !exclude_staged, false);
                    c.set("quantity", Rv::Num((max - spent).max(0) as f64));
                    c.set("spent", Rv::Num(spent as f64));
                    c.set("spentInTree", Rv::Num(in_tree as f64));
                }
                c
            })
            .collect();
        Some(Rv::list_of(items))
    }

    /// The tree a condition gates: its first node's in the config.
    fn condition_tree(&self, config: i64, cond: i64) -> Option<i64> {
        self.nodes(config).into_iter().find_map(|(t, n)| {
            let node = self.recorded_node(config, n)?;
            node.nums("conditionIDs").contains(&cond).then_some(t)
        })
    }

    /// `C_Traits.GetConditionInfo(config, cond)`: a spending condition's
    /// points still to spend in its tree (its points: what the recording
    /// still lacked and had spent).
    pub fn condition(&self, config: i64, cond: i64) -> Option<Rv> {
        if let Some(client) = &self.client {
            // The client's: its points to spend in its tree (unverified:
            // its other requirements, quests, achievements, levels; camelot's
            // conditions have none).
            let c = client.conditions.get(&uid(cond))?;
            let mut f = vec![
                ("condID", num(uid(cond))),
                ("isAlwaysMet", Rv::Bool(c.flags & IS_ALWAYS_MET != 0)),
                ("isGate", Rv::Bool(c.flags & IS_GATE != 0)),
                ("isMet", Rv::Bool(true)),
                ("isSufficient", Rv::Bool(c.flags & IS_SUFFICIENT != 0)),
                ("type", num(c.kind)),
            ];
            if c.currency == 0 {
                return Some(fields(f));
            }
            f.push(("traitCurrencyID", num(c.currency)));
            f.push(("spentAmountRequired", num(c.spent)));
            if let Some(t) = &c.tooltip {
                f.push(("tooltipFormat", Rv::Str(t.clone())));
            }
            // In its node group (a tab: recorded, 266 of 266 conditions),
            // else its tree.
            let group = client.groups.get(&c.group);
            let now =
                self.spent_where(
                    config,
                    i64::from(c.currency),
                    true,
                    false,
                    |t, n| match group {
                        Some(nodes) if c.group != 0 => nodes.contains(&uid(n)),
                        _ => t == i64::from(c.tree),
                    },
                );
            return Some(Self::still_to_spend(fields(f), i64::from(c.spent) - now));
        }
        let info = self.first(&format!("C_Traits.GetConditionInfo({config}, {cond})"))?;
        let (Some(currency), Some(remaining)) =
            (info.num("traitCurrencyID"), info.num("spentAmountRequired"))
        else {
            return Some(info);
        };
        let Some(tree) = self.condition_tree(config, cond) else {
            return Some(info);
        };
        let currency = currency as i64;
        let needed = remaining as i64 + self.spent(config, currency, Some(tree), false, true);
        let now = self.spent(config, currency, Some(tree), true, false);
        Some(Self::still_to_spend(info, needed - now))
    }

    /// A spending condition with `left` points still to spend: met at 0
    /// (without its tooltip), else its tooltip's text with the points.
    fn still_to_spend(mut info: Rv, left: i64) -> Rv {
        let left = left.max(0);
        info.set("isMet", Rv::Bool(left == 0));
        info.set("spentAmountRequired", Rv::Num(left as f64));
        if left == 0 {
            info.remove("tooltipFormat");
            info.remove("tooltipText");
        } else if let Some(Rv::Str(format)) = info.get("tooltipFormat").cloned() {
            let text = format
                .replacen("%d", &left.to_string(), 1)
                .replacen("%s", "", 1);
            info.set("tooltipText", Rv::Str(format!("|cffff2020{text}|r")));
        }
        info
    }

    fn condition_met(&self, config: i64, cond: i64) -> bool {
        self.condition(config, cond).is_some_and(|c| {
            c.num("type") != Some(AVAILABLE) || c.flag("isMet") || c.flag("isAlwaysMet")
        })
    }

    /// An edge's source at its last rank.
    fn satisfies(&self, config: i64, node: i64) -> bool {
        self.rank(config, node, true) >= self.max_ranks(config, node)
    }

    /// The edges into a node: (source, type).
    fn edges_into(&self, config: i64, node: i64) -> Vec<(i64, f64)> {
        // The client's: the edges of the nodes of its tree.
        if let Some(client) = &self.client {
            let Some(tree) = self.shape(node).and_then(|n| client.trees.get(&n.tree)) else {
                return Vec::new();
            };
            return tree
                .nodes
                .iter()
                .filter_map(|s| Some((*s, client.nodes.get(s)?)))
                .flat_map(|(s, n)| {
                    n.edges
                        .iter()
                        .filter(|e| e.target == uid(node))
                        .map(move |e| (i64::from(s), f64::from(e.kind)))
                })
                .collect();
        }
        let mut out = Vec::new();
        for (_, source) in self.nodes(config) {
            let Some(info) = self.recorded_node(config, source) else {
                continue;
            };
            for edge in info
                .get("visibleEdges")
                .map(|e| e.list())
                .unwrap_or_default()
            {
                if edge.num("targetNode") == Some(node as f64) {
                    out.push((source, edge.num("type").unwrap_or(0.0)));
                }
            }
        }
        out
    }

    fn meets_edges(&self, config: i64, node: i64) -> bool {
        let edges = self.edges_into(config, node);
        let sufficient: Vec<i64> = edges
            .iter()
            .filter(|(_, t)| *t == SUFFICIENT)
            .map(|(s, _)| *s)
            .collect();
        let required = edges
            .iter()
            .filter(|(_, t)| *t == REQUIRED)
            .map(|(s, _)| *s);
        (sufficient.is_empty() || sufficient.iter().any(|s| self.satisfies(config, *s)))
            && required.into_iter().all(|s| self.satisfies(config, s))
    }

    fn cannot_refund(&self, config: i64, node: i64) -> bool {
        if let Some(client) = &self.client {
            return self
                .shape(node)
                .and_then(|n| client.trees.get(&n.tree))
                .is_some_and(|t| t.flags & CANNOT_REFUND != 0);
        }
        self.tree_of(config, node)
            .and_then(|t| self.first(&format!("C_Traits.GetTreeInfo({config}, {t})")))
            .is_some_and(|t| t.flag("cannotRefund"))
    }

    /// The class and config recorded holding a node (the tree's owner's).
    fn host(&self, node: i64) -> Option<(String, i64)> {
        let suffix = format!(", {node})");
        let found = |call: &str| -> Option<i64> {
            call.strip_prefix("C_Traits.GetNodeInfo(")?
                .strip_suffix(&suffix)?
                .parse()
                .ok()
        };
        let mine = data()
            .keys()
            .filter(|(class, _)| *class == self.class)
            .find_map(|(class, call)| found(call).map(|c| (class.to_string(), c)));
        mine.or_else(|| {
            data()
                .keys()
                .find_map(|(class, call)| found(call).map(|c| (class.to_string(), c)))
        })
    }

    /// `C_Traits.GetNodeInfo(config, node)`: the recording's, its ranks'
    /// answers worked out. Asked through a config without the node: the
    /// node as with nothing of its tree bought, never to buy (recorded: the
    /// druid's class config asked of its legacy nodes).
    pub fn node(&self, config: i64, node: i64) -> Option<Rv> {
        // A node of a tree the config does not hold (recorded so, or not).
        if !self.bare && self.tree_of(config, node).is_none() {
            // The client's node needs no host: its tree is its own.
            let (class, host) = match self.recorded_node(config, node) {
                _ if self.client.is_some() => (self.class.clone(), config),
                Some(_) => (self.class.clone(), config),
                None => self.host(node)?,
            };
            let bare = Model {
                class,
                state: self.state,
                bare: true,
                points: None,
                client: self.client.clone(),
            };
            let mut info = bare.node(host, node)?;
            info.set("canPurchaseRank", Rv::Bool(false));
            info.set("canRefundRank", Rv::Bool(false));
            return Some(info);
        }
        let mut info = self.node_info(config, node)?;
        let rank = self.rank(config, node, true);
        let max = self.max_ranks(config, node);
        let available = info
            .nums("conditionIDs")
            .into_iter()
            .all(|c| self.condition_met(config, c));
        let meets = self.meets_edges(config, node);
        let entries = info.nums("entryIDs");
        let tiered = info.num("type") == Some(TIERED);
        // A node's entry at a rank, and the rank within it: a tiered
        // node's entries one after another (recorded: rank 5 of five
        // one-rank entries is the fifth at 1; rank 0 the first at 0), else
        // the node's entry (the one it shows).
        // (the recorded choice of a node of entries to choose from)
        let shown = self
            .recorded_node(config, node)
            .filter(|_| !self.bare)
            .unwrap_or_else(|| info.clone())
            .get("activeEntry")
            .and_then(|e| e.num("entryID"))
            .map(|e| e as i64)
            .or_else(|| entries.first().copied());
        let at = |r: i64| -> Option<(i64, i64)> {
            if !tiered || r == 0 {
                return shown.map(|e| (e, r));
            }
            let mut left = r;
            for &e in &entries {
                let m = self.entry_max_ranks(config, e);
                if left <= m {
                    return Some((e, left));
                }
                left -= m;
            }
            None
        };
        let pair = |(e, r): (i64, i64)| {
            Rv::Table(vec![
                ("entryID".into(), Rv::Num(e as f64)),
                ("rank".into(), Rv::Num(r as f64)),
            ])
        };
        for key in ["activeRank", "currentRank", "ranksPurchased"] {
            info.set(key, Rv::Num(rank as f64));
        }
        if let Some(active) = at(rank) {
            info.set("activeEntry", pair(active));
        }
        match at(rank + 1).filter(|_| rank < max) {
            Some(next) => info.set("nextEntry", pair(next)),
            None => info.remove("nextEntry"),
        }
        let committed = self.start_rank(config, node);
        let with = match at(committed).filter(|_| committed > 0) {
            Some((e, _)) => vec![Rv::Num(e as f64)],
            None => Vec::new(),
        };
        info.set("entryIDsWithCommittedRanks", Rv::list_of(with));
        info.set("isAvailable", Rv::Bool(available));
        info.set("meetsEdgeRequirements", Rv::Bool(meets));
        // Whatever the points left (recorded: the druid's spent class
        // tree's nodes can be bought); never a node flagged so.
        let never = info.num("flags").unwrap_or(0.0) as i64 & NEVER_PURCHASABLE != 0;
        info.set(
            "canPurchaseRank",
            Rv::Bool(available && meets && rank < max && !never),
        );
        // A rank bought this session can be taken back, whatever the tree
        // (recorded: the shaman's, its tree `cannotRefund`, staged and just
        // applied; the next session not).
        let refund =
            rank > self.start_rank(config, node) || (rank > 0 && !self.cannot_refund(config, node));
        info.set("canRefundRank", Rv::Bool(refund));
        let active = self.satisfies(config, node);
        if let Some(Rv::Table(edges)) = info.get("visibleEdges").cloned() {
            let edges = edges
                .into_iter()
                .map(|(k, mut e)| {
                    e.set("isActive", Rv::Bool(active));
                    (k, e)
                })
                .collect();
            info.set("visibleEdges", Rv::Table(edges));
        }
        Some(info)
    }

    /// The trees of the config paying in a currency, in its order.
    fn trees_paying(&self, config: i64, currency: i64) -> Vec<i64> {
        if currency == CLASS_CURRENCY {
            return CLASS_TREES.to_vec();
        }
        self.trees(config)
            .into_iter()
            .filter(|t| {
                self.first(&format!(
                    "C_Traits.GetTreeCurrencyInfo({config}, {t}, false)"
                ))
                .is_some_and(|l| {
                    l.list()
                        .iter()
                        .any(|c| c.num("traitCurrencyID") == Some(currency as f64))
                })
            })
            .collect()
    }

    /// The edges' targets of a node.
    fn targets(&self, config: i64, node: i64) -> Vec<i64> {
        if self.client.is_some() {
            return self
                .shape(node)
                .map(|n| n.edges.iter().map(|e| i64::from(e.target)).collect())
                .unwrap_or_default();
        }
        self.recorded_node(config, node)
            .and_then(|i| i.get("visibleEdges").cloned())
            .map(|e| {
                e.list()
                    .iter()
                    .filter_map(|e| e.num("targetNode").map(|n| n as i64))
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The character whose recorded trait data is the player's: the test's
/// (`e.setTraitCharacter`), else the class's first recorded one.
fn character(vm: &Vm) -> String {
    let traits = &engine(vm).world.traits;
    if let Some(c) = &traits.character {
        return c.clone();
    }
    let prefix = format!("{}-", engine(vm).units.list[PLAYER.0].class.to_uppercase());
    // The first of the class's, in order: those with the prefix start at
    // the first key not before it.
    let keys = characters();
    let at = keys.partition_point(|k| *k < prefix.as_str());
    keys.get(at)
        .filter(|k| k.starts_with(&prefix))
        .map_or(prefix, |k| k.to_string())
}

/// The recorded characters' keys, in order, each once.
fn characters() -> &'static [&'static str] {
    static KEYS: OnceLock<Vec<&'static str>> = OnceLock::new();
    KEYS.get_or_init(|| {
        let mut keys: Vec<&str> = data().keys().map(|(k, _)| *k).collect();
        keys.sort_unstable();
        keys.dedup();
        keys
    })
}

fn model(vm: &Vm) -> Model<'_> {
    let key = character(vm);
    Model::of(vm, &key)
}

/// The inspected config (`-1`): the recorded character the test named
/// and its class config.
fn inspected(vm: &mut Vm) -> Option<(String, i64)> {
    let config = crate::inspect::trait_character(vm)?;
    let suffix = format!("-{config}");
    let key = data()
        .keys()
        .map(|(k, _)| *k)
        .find(|k| k.ends_with(&suffix))?;
    Some((key.to_string(), config))
}

/// A config asked of: the inspected's (`-1`) as its recorded character's
/// own, else the player's (whether it is the inspected's).
fn resolve(vm: &mut Vm, config: i64) -> (String, i64, bool) {
    if config == INSPECT
        && let Some((key, real)) = inspected(vm)
    {
        return (key, real, true);
    }
    (character(vm), config, false)
}

/// The inspected config's ID.
const INSPECT: i64 = -1;

/// The default character's own combat config (`stats.combatConfig` of
/// `data/camelot-default-character.json`, from another recording than the
/// talent model's).
const DEFAULT_CHARACTER_CONFIG: f64 = 16273023.0;

/// The player's talent config as the model has it (decision T328): a
/// config the class's recorded character's configs do not hold (the default
/// character's own, from another recording) is the recorded character's
/// active one, so the player's active and combat configs name the tree the
/// model answers for; a config the model holds, or a class never recorded,
/// as it is.
pub(crate) fn player_config(vm: &Vm, config: f64) -> f64 {
    // Only the default character's own config: a character given with its
    // own snapshot (a scenario's paladin's 73810) keeps the config it has.
    if config != DEFAULT_CHARACTER_CONFIG {
        return config;
    }
    let m = model(vm);
    if !m.known()
        || m.recorded(&format!("C_Traits.GetConfigInfo({})", config as i64))
            .is_some()
    {
        return config;
    }
    match m.first("C_ClassTalents.GetActiveConfigID()") {
        Some(Rv::Num(own)) => own,
        _ => config,
    }
}

fn traits_mut(vm: &mut Vm) -> &mut Traits {
    &mut engine_mut(vm).world.traits
}

fn fire(vm: &mut Vm, e: &str, args: &[Value]) {
    widgets::fire_event(vm, e, args);
}

fn int(vm: &mut Vm, c: &NativeCall, i: usize) -> Option<i64> {
    match vm.hidden(c.arg(i)) {
        Value::Num(n) => Some(n as i64),
        _ => None,
    }
}

fn answer(vm: &mut Vm, rv: Option<Rv>) -> Vec<Value> {
    rv.map(|v| vec![v.to_value(vm)]).unwrap_or_default()
}

/// Any other call of a recorded class: its recorded answer; a class
/// never recorded: the general answers.
fn recorded(vm: &mut Vm, c: NativeCall) -> Ret {
    let Some(name) = crate::recorded::called_name(vm, &c) else {
        return Ok(vec![]);
    };
    // The active config: the active spec group's combat config, as the
    // character's stats have it (recorded: 3 of 3 characters asked, the
    // shaman's 5999541, the druid's 22147 and the paladin's 73810, each
    // the snapshot's `GetCombatConfigIDForSpecGroup(1)`); else the
    // recorded character's.
    if name == "C_ClassTalents.GetActiveConfigID"
        && let Some(id) = crate::stats::active_combat_config(vm)
    {
        return Ok(vec![Value::Num(id)]);
    }
    let mut call = crate::call_text(vm, &name, &c, 1);
    let mut key = character(vm);
    // The inspected config's calls: its recorded character's own.
    if call.contains("(-1")
        && let Some((k, real)) = inspected(vm)
    {
        call = call.replacen("(-1", &format!("({real}"), 1);
        key = k;
    }
    let m = Model::of(vm, &key);
    // The trees' structure: the client's where its tables are at hand.
    if m.known()
        && let Some(rv) = m.client_answer(&name, &call)
    {
        return Ok(answer(vm, rv));
    }
    if m.known()
        && let Some(values) = m.recorded(&call)
    {
        return Ok(values.iter().map(|v| v.to_value(vm)).collect());
    }
    if let Some(values) = unknown_tree(vm, &name, &call) {
        return Ok(values);
    }
    crate::recorded::recorded(vm, c)
}

/// A tree or config the client lacks (the full probe, 1.60.1: IDs -1, 0,
/// 1, 1.5 and 2147483647): no nodes, no increased data, the hash of
/// nothing (MD5 of no bytes), no edits ("You can't do that right now.",
/// `ERR_CLIENT_LOCKED_OUT`), variation 0. Not for a call the recordings
/// answer.
fn unknown_tree(vm: &mut Vm, name: &str, call: &str) -> Option<Vec<Value>> {
    if crate::recorded::answers().contains_key(call) {
        return None;
    }
    Some(match name {
        "C_Traits.GetTreeNodes" | "C_Traits.GetIncreasedTraitData" => {
            vec![Value::Table(vm.new_table())]
        }
        "C_Traits.GetTreeHash" => {
            const EMPTY_MD5: [u8; 16] = [
                212, 29, 140, 217, 143, 0, 178, 4, 233, 128, 9, 152, 236, 248, 66, 126,
            ];
            let t = vm.new_table();
            for (i, b) in EMPTY_MD5.iter().enumerate() {
                let _ = vm
                    .heap
                    .table_mut(t)
                    .set(Value::Num(i as f64 + 1.0), Value::Num(f64::from(*b)));
            }
            vec![Value::Table(t)]
        }
        "C_Traits.CanEditConfig" => {
            vec![Value::Bool(false), vm.str("You can't do that right now.")]
        }
        "C_Traits.GetConfigVariationID" => vec![Value::Num(0.0)],
        _ => return None,
    })
}

/// A behaviour of the model for a recorded class, else the general one.
macro_rules! modelled {
    ($vm:ident, $c:ident, $body:expr) => {{
        if !model($vm).known() {
            return crate::recorded::recorded($vm, $c);
        }
        $body
    }};
}

fn get_node_info(vm: &mut Vm, c: NativeCall) -> Ret {
    modelled!(vm, c, {
        let (Some(config), Some(node)) = (int(vm, &c, 1), int(vm, &c, 2)) else {
            return Ok(vec![]);
        };
        let (key, config, viewed) = resolve(vm, config);
        let mut rv = Model::of(vm, &key).node(config, node);
        // Another's talents viewed: nothing to buy or take back.
        if viewed && let Some(info) = rv.as_mut() {
            info.set("canPurchaseRank", Rv::Bool(false));
            info.set("canRefundRank", Rv::Bool(false));
        }
        Ok(answer(vm, rv))
    })
}

fn get_condition_info(vm: &mut Vm, c: NativeCall) -> Ret {
    modelled!(vm, c, {
        let (Some(config), Some(cond)) = (int(vm, &c, 1), int(vm, &c, 2)) else {
            return Ok(vec![]);
        };
        let (key, config, _) = resolve(vm, config);
        let rv = Model::of(vm, &key).condition(config, cond);
        Ok(answer(vm, rv))
    })
}

fn get_tree_currency_info(vm: &mut Vm, c: NativeCall) -> Ret {
    modelled!(vm, c, {
        let (Some(config), Some(tree)) = (int(vm, &c, 1), int(vm, &c, 2)) else {
            return Ok(vec![]);
        };
        let exclude = vm.hidden(c.arg(3)).truthy();
        if config == INSPECT && inspected(vm).is_none() {
            // No recorded tree either: no currencies (the full probe,
            // 1.60.1: `GetTreeCurrencyInfo(-1, 1, true)` {}).
            let Some(rv) = bare_tree_currency(vm, tree, exclude) else {
                return Ok(vec![Value::Table(vm.new_table())]);
            };
            return Ok(answer(vm, Some(rv)));
        }
        let (key, config, viewed) = resolve(vm, config);
        // The player's own talents: the points their level earned.
        let points = (!viewed).then(|| crate::stats::talent_points(vm) as i64);
        let mut m = Model::of(vm, &key);
        if let Some(p) = points {
            m = m.with_points(p);
        }
        let rv = m.tree_currency(config, tree, exclude);
        // A tree the client lacks: no currencies (the full probe, 1.60.1).
        if rv.is_none() {
            return Ok(vec![Value::Table(vm.new_table())]);
        }
        Ok(answer(vm, rv))
    })
}

/// An inspected player's tree without its recorded character (the test
/// named none): its currency as the tree's owner's with nothing bought
/// (as its nodes, `Model::node`; unverified: its points then all
/// unspent).
fn bare_tree_currency(vm: &Vm, tree: i64, exclude: bool) -> Option<Rv> {
    let wanted = |call: &str| -> Option<i64> {
        call.strip_prefix("C_Traits.GetTreeCurrencyInfo(")?
            .strip_suffix(&format!(", {tree}, false)"))?
            .parse()
            .ok()
    };
    let mut hosts: Vec<(&str, i64)> = data()
        .keys()
        .filter_map(|(class, call)| Some((*class, wanted(call)?)))
        .collect();
    // The tree's owner: a config holding it (every class's config answers
    // of the class trees); the same on every run (the data's order is a
    // hash map's).
    hosts.sort_unstable();
    let owns = |(class, config): &(&str, i64)| Model::of(vm, class).trees(*config).contains(&tree);
    let (class, config) = hosts.iter().find(|h| owns(h)).or(hosts.first()).copied()?;
    let bare = Model {
        class: class.to_string(),
        state: &engine(vm).world.traits,
        bare: true,
        points: None,
        client: crate::clientdata::traits(vm),
    };
    bare.tree_currency(config, tree, exclude)
}

fn can_purchase_rank(vm: &mut Vm, c: NativeCall) -> Ret {
    modelled!(vm, c, {
        let (Some(config), Some(node)) = (int(vm, &c, 1), int(vm, &c, 2)) else {
            return Ok(vec![Value::Bool(false)]);
        };
        let can = model(vm)
            .node(config, node)
            .is_some_and(|n| n.flag("canPurchaseRank"));
        Ok(vec![Value::Bool(can)])
    })
}

fn can_refund_rank(vm: &mut Vm, c: NativeCall) -> Ret {
    modelled!(vm, c, {
        let (Some(config), Some(node)) = (int(vm, &c, 1), int(vm, &c, 2)) else {
            return Ok(vec![Value::Bool(false)]);
        };
        let can = model(vm)
            .node(config, node)
            .is_some_and(|n| n.flag("canRefundRank"));
        Ok(vec![Value::Bool(can)])
    })
}

fn staged_nodes(vm: &Vm, config: i64) -> Vec<i64> {
    let m = model(vm);
    m.nodes(config)
        .into_iter()
        .map(|(_, n)| n)
        .filter(|n| m.staged(config, *n) > 0)
        .collect()
}

fn config_has_staged_changes(vm: &mut Vm, c: NativeCall) -> Ret {
    modelled!(vm, c, {
        let Some(config) = int(vm, &c, 1) else {
            return Ok(vec![Value::Bool(false)]);
        };
        let staged = !staged_nodes(vm, config).is_empty();
        Ok(vec![Value::Bool(staged)])
    })
}

/// `C_Traits.GetStagedChanges(config)`: the nodes with ranks bought,
/// refunded, swapped (none recorded staged: as documented).
fn get_staged_changes(vm: &mut Vm, c: NativeCall) -> Ret {
    modelled!(vm, c, {
        let Some(config) = int(vm, &c, 1) else {
            return Ok(vec![]);
        };
        let nodes = staged_nodes(vm, config);
        if nodes.is_empty() {
            return Ok(vec![]);
        }
        let list = Rv::list_of(nodes.into_iter().map(|n| Rv::Num(n as f64)).collect());
        let purchases = list.to_value(vm);
        let refunds = Value::Table(vm.new_table());
        let swaps = Value::Table(vm.new_table());
        Ok(vec![purchases, refunds, swaps])
    })
}

/// The nodes' answers of a config, to tell which a change touched.
fn snapshot(vm: &Vm, config: i64) -> Vec<(i64, Option<Rv>)> {
    let m = model(vm);
    m.nodes(config)
        .into_iter()
        .map(|(_, n)| (n, m.node(config, n)))
        .collect()
}

/// `C_Traits.PurchaseRank(config, node)`: staged at once, its events.
fn purchase_rank(vm: &mut Vm, c: NativeCall) -> Ret {
    modelled!(vm, c, {
        let (Some(config), Some(node)) = (int(vm, &c, 1), int(vm, &c, 2)) else {
            return Ok(vec![Value::Bool(false)]);
        };
        let can = model(vm)
            .node(config, node)
            .is_some_and(|n| n.flag("canPurchaseRank"));
        if !can {
            return Ok(vec![Value::Bool(false)]);
        }
        let before = snapshot(vm, config);
        *traits_mut(vm).staged.entry((config, node)).or_insert(0) += 1;
        let after = snapshot(vm, config);
        let (targets, currencies) = {
            let m = model(vm);
            let currencies: Vec<i64> = m
                .cost(config, node)
                .into_iter()
                .flat_map(|(cur, _)| m.trees_paying(config, cur))
                .collect();
            (m.targets(config, node), currencies)
        };
        fire(vm, "TRAIT_NODE_CHANGED", &[Value::Num(node as f64)]);
        for t in &targets {
            fire(vm, "TRAIT_NODE_CHANGED", &[Value::Num(*t as f64)]);
        }
        let mut seen = Vec::new();
        for tree in currencies {
            if !seen.contains(&tree) {
                seen.push(tree);
                fire(
                    vm,
                    "TRAIT_TREE_CURRENCY_INFO_UPDATED",
                    &[Value::Num(tree as f64)],
                );
            }
        }
        // The nodes it unlocked, after the currencies, then its edges'
        // targets again (recorded: the legacy druid's fifth point). Those
        // with edges of their own after the others, each in the tree's
        // order (recorded once: 110303 and 110305, then 110304 with its
        // edge; unverified, as is any order key the one sample fits).
        let mut unlocked: Vec<i64> = before
            .iter()
            .zip(after.iter())
            .filter(|((n, a), (_, b))| *n != node && !targets.contains(n) && a != b)
            .map(|((n, _), _)| *n)
            .collect();
        {
            let m = model(vm);
            unlocked.sort_by_key(|n| !m.targets(config, *n).is_empty());
        }
        if !unlocked.is_empty() {
            for n in unlocked {
                fire(vm, "TRAIT_NODE_CHANGED", &[Value::Num(n as f64)]);
            }
            for t in targets {
                fire(vm, "TRAIT_NODE_CHANGED", &[Value::Num(t as f64)]);
            }
        }
        Ok(vec![Value::Bool(true)])
    })
}

fn config_of(c: &NativeCall) -> i64 {
    match c.upvals.first() {
        Some(Value::Num(n)) => *n as i64,
        _ => 0,
    }
}

/// `C_Traits.CommitConfig(config)`: the server's word a round trip later
/// (`SPELLS_CHANGED`), the ranks applied after.
fn commit_config(vm: &mut Vm, c: NativeCall) -> Ret {
    modelled!(vm, c, {
        let Some(config) = int(vm, &c, 1) else {
            return Ok(vec![Value::Bool(false)]);
        };
        commit(vm, config)
    })
}

/// `C_ClassTalents.CommitConfig(savedConfigID)`: the active config's
/// staged ranks applied (the loadout saved: unverified).
fn commit_class_config(vm: &mut Vm, c: NativeCall) -> Ret {
    modelled!(vm, c, {
        let Some(config) = model(vm)
            .first("C_ClassTalents.GetActiveConfigID()")
            .and_then(|v| match v {
                Rv::Num(n) => Some(n as i64),
                _ => None,
            })
        else {
            return Ok(vec![Value::Bool(false)]);
        };
        commit(vm, config)
    })
}

/// A config's staged ranks sent to the server.
fn commit(vm: &mut Vm, config: i64) -> Ret {
    if staged_nodes(vm, config).is_empty() {
        return Ok(vec![Value::Bool(false)]);
    }
    let delay = crate::clock::server_delay(vm);
    let native = vm.native_with(
        "traitsCommitted",
        committed as NativeFn,
        vec![Value::Num(config as f64)],
    );
    crate::clock::after_engine(vm, delay, native);
    Ok(vec![Value::Bool(true)])
}

fn committed(vm: &mut Vm, c: NativeCall) -> Ret {
    crate::batch::spells_changed(vm);
    let generic = model(vm)
        .first(&format!("C_Traits.GetConfigInfo({})", config_of(&c)))
        .and_then(|i| i.num("type"))
        == Some(GENERIC);
    let after = if generic {
        COMMIT_AFTER_GENERIC
    } else {
        COMMIT_AFTER
    };
    let native = vm.native_with("traitsApplied", applied as NativeFn, c.upvals.to_vec());
    crate::clock::after_engine(vm, after, native);
    Ok(vec![])
}

fn applied(vm: &mut Vm, c: NativeCall) -> Ret {
    let config = config_of(&c);
    let nodes = staged_nodes(vm, config);
    for n in &nodes {
        let rank = model(vm).rank(config, *n, true);
        let t = traits_mut(vm);
        t.staged.remove(&(config, *n));
        t.committed.insert((config, *n), rank);
    }
    let mods: usize = nodes
        .iter()
        .flat_map(|n| model(vm).node_spells(config, *n))
        .filter_map(|s| DAMAGE_DONE_MODS.iter().find(|(id, _)| *id == s))
        .map(|(_, count)| count)
        .sum();
    for _ in 0..mods {
        crate::units::fire_unit(vm, "PLAYER_DAMAGE_DONE_MODS", PLAYER, &[]);
    }
    for n in nodes {
        fire(vm, "TRAIT_NODE_CHANGED", &[Value::Num(n as f64)]);
    }
    for tree in model(vm).trees(config) {
        fire(
            vm,
            "TRAIT_TREE_CURRENCY_INFO_UPDATED",
            &[Value::Num(tree as f64)],
        );
    }
    fire(vm, "TRAIT_CONFIG_UPDATED", &[Value::Num(config as f64)]);
    fire(vm, "PLAYER_TALENT_UPDATE", &[]);
    let native = vm.native_with("traitsBuffs", buffs as NativeFn, vec![]);
    crate::clock::after_engine(vm, BUFFS_AFTER, native);
    Ok(vec![])
}

fn buffs(vm: &mut Vm, _c: NativeCall) -> Ret {
    crate::character::group_buffs_changed(vm);
    Ok(vec![])
}

/// `C_Traits.RollbackConfig(config)`: the staged ranks dropped
/// (unverified: its events).
fn rollback_config(vm: &mut Vm, c: NativeCall) -> Ret {
    modelled!(vm, c, {
        let Some(config) = int(vm, &c, 1) else {
            return Ok(vec![Value::Bool(false)]);
        };
        let nodes = staged_nodes(vm, config);
        for n in &nodes {
            traits_mut(vm).staged.remove(&(config, *n));
        }
        for n in nodes {
            fire(vm, "TRAIT_NODE_CHANGED", &[Value::Num(n as f64)]);
        }
        Ok(vec![Value::Bool(true)])
    })
}

/// `C_Traits.GetConfigInfo(config)`: the inspected's (-1), else the class's.
fn get_config_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(r) = crate::inspect::config_info(vm, &c) {
        return Ok(r);
    }
    recorded(vm, c)
}

/// `C_MajorFactions.GetCurrentRenownLevel(faction)`: the test's
/// (`e.setRenown`), else the recorded answers.
fn get_current_renown_level(vm: &mut Vm, c: NativeCall) -> Ret {
    if let Some(faction) = int(vm, &c, 1)
        && let Some(level) = engine(vm).world.traits.renown.get(&faction).copied()
    {
        return Ok(vec![Value::Num(level)]);
    }
    crate::recorded::recorded(vm, c)
}

/// WoW Forever's PvP ranks: major faction 2800 (Blizzard's
/// `PVP_RANK_POINTS_FACTION_ID`), probed 2026-10-24: 14 ranks, the
/// reputation to reach ranks 1 to 4 750, 1650, 2700 and 3900, rank 14
/// 24750 (the PvP tab's "up to a maximum of 24750 for Rank 14"). Ranks 1
/// to 4 are 75·n·(n + 9); ranks 5 to 13 are taken so (unverified: it gives
/// 24150 for rank 14).
const PVP_RANKS: i64 = 2800;
const PVP_MAX_RANK: i64 = 14;

/// The reputation to reach PvP rank `level` (0 at rank 0).
fn pvp_rank_total(level: i64) -> f64 {
    let n = level.clamp(0, PVP_MAX_RANK);
    if n == PVP_MAX_RANK {
        return 24750.0;
    }
    75.0 * (n * (n + 9)) as f64
}

fn renown_level(vm: &mut Vm, faction: i64) -> f64 {
    engine(vm)
        .world
        .traits
        .renown
        .get(&faction)
        .copied()
        .unwrap_or(0.0)
}

/// `C_MajorFactions.GetMajorFactionProgressionInfo(faction)`: for the PvP
/// ranks, the player's rank (the test's, `e.setRenown`, else 0) and the
/// reputation to the next, as the client answered a rank 0 character
/// (its weekly fields 0, the week -1); else the recorded answers.
fn get_major_faction_progression_info(vm: &mut Vm, c: NativeCall) -> Ret {
    if int(vm, &c, 1) != Some(PVP_RANKS) {
        return crate::recorded::recorded(vm, c);
    }
    let level = renown_level(vm, PVP_RANKS);
    let threshold = pvp_rank_total(level as i64 + 1) - pvp_rank_total(level as i64);
    let t = vm.new_table();
    for (k, v) in [
        ("currentWeekProgressiveMaxLevel", 0.0),
        ("maxLevel", PVP_MAX_RANK as f64),
        ("renownReputationEarned", 0.0),
        ("weekNumber", -1.0),
        ("renownLevel", level),
        ("previousWeekProgressiveMaxLevel", 0.0),
        ("renownLevelThreshold", threshold),
    ] {
        let key = vm.str(k);
        let _ = vm.heap.table_mut(t).set(key, Value::Num(v));
    }
    Ok(vec![Value::Table(t)])
}

/// `C_MajorFactions.GetTotalReputationForRenownLevel(faction, level)`:
/// the PvP ranks' totals; else the recorded answers.
fn get_total_reputation_for_renown_level(vm: &mut Vm, c: NativeCall) -> Ret {
    if int(vm, &c, 1) != Some(PVP_RANKS) {
        return crate::recorded::recorded(vm, c);
    }
    let level = int(vm, &c, 2).unwrap_or(0);
    Ok(vec![Value::Num(pvp_rank_total(level))])
}

/// `C_MajorFactions.GetRenownRewardsForLevel(faction, level)`: rank 1's
/// faction tabard for the PvP ranks (probed), none for their other ranks
/// (unverified); else the recorded answers.
fn get_renown_rewards_for_level(vm: &mut Vm, c: NativeCall) -> Ret {
    if int(vm, &c, 1) != Some(PVP_RANKS) {
        return crate::recorded::recorded(vm, c);
    }
    let list = vm.new_table();
    if int(vm, &c, 2) == Some(1) {
        let r = vm.new_table();
        let strings = [
            ("description", "Faction Tabard"),
            ("toastDescription", "Faction Tabard Unlocked"),
            ("name", "Rank 1 Rewards"),
        ];
        for (k, v) in strings {
            let (key, v) = (vm.str(k), vm.str(v));
            let _ = vm.heap.table_mut(r).set(key, v);
        }
        for (k, v) in [
            ("icon", Value::Num(135026.0)),
            ("isAccountUnlock", Value::Bool(false)),
            ("uiOrder", Value::Num(0.0)),
            ("renownRewardID", Value::Num(0.0)),
        ] {
            let key = vm.str(k);
            let _ = vm.heap.table_mut(r).set(key, v);
        }
        let _ = vm
            .heap
            .table_mut(list)
            .set(Value::Num(1.0), Value::Table(r));
    }
    Ok(vec![Value::Table(list)])
}

/// `e.setTraitRanks(config, { [node] = rank })`: the ranks committed as
/// the session begins.
pub fn control_set_trait_ranks(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(config), Value::Table(t)) = (c.arg(1), c.arg(2)) else {
        return Err(vm.error("setTraitRanks: a config and a table of ranks by node expected"));
    };
    let mut ranks = Vec::new();
    let mut key = Value::Nil;
    while let Ok(Some((k, v))) = vm.heap.table(t).next(key) {
        if let (Value::Num(node), Value::Num(rank)) = (k, v) {
            ranks.push((node as i64, rank as i64));
        }
        key = k;
    }
    let state = traits_mut(vm);
    for (node, rank) in ranks {
        state.committed.insert((config as i64, node), rank);
        state.start.insert((config as i64, node), rank);
    }
    Ok(vec![])
}

/// `e.setTraitCharacter(configID)`: the recorded character (named by its
/// class config's ID) whose trait data is the player's.
pub fn control_set_trait_character(vm: &mut Vm, c: NativeCall) -> Ret {
    let config = vm.check_number(&c, 1)? as i64;
    let suffix = format!("-{config}");
    let Some(key) = data()
        .keys()
        .map(|(k, _)| *k)
        .find(|k| k.ends_with(&suffix))
    else {
        return Err(vm.error(&format!(
            "setTraitCharacter: no character recorded with config {config}"
        )));
    };
    traits_mut(vm).character = Some(key.to_string());
    Ok(vec![])
}

/// `e.setRenown(faction, level, { events? })`: a major faction's renown;
/// with `events`, a change told as the client tells it:
/// `MAJOR_FACTION_RENOWN_LEVEL_CHANGED(faction, new, old)` (the
/// documentation's arguments; unverified in the client).
pub fn control_set_renown(vm: &mut Vm, c: NativeCall) -> Ret {
    let (Value::Num(faction), Value::Num(level)) = (c.arg(1), c.arg(2)) else {
        return Err(vm.error("setRenown: a faction ID and a level expected"));
    };
    let events = match c.arg(3) {
        Value::Table(o) => {
            let k = vm.str("events");
            vm.heap.table(o).get(k).truthy()
        }
        _ => false,
    };
    let old = traits_mut(vm)
        .renown
        .insert(faction as i64, level)
        .unwrap_or(0.0);
    if events && old != level {
        crate::widgets::fire_event(
            vm,
            "MAJOR_FACTION_RENOWN_LEVEL_CHANGED",
            &[Value::Num(faction), Value::Num(level), Value::Num(old)],
        );
    }
    Ok(vec![])
}

/// The functions every recorded class answers as recorded.
fn recorded_names() -> Vec<&'static str> {
    let mut names: Vec<&'static str> = data()
        .keys()
        .filter_map(|(_, call)| call.split('(').next())
        .collect();
    names.sort_unstable();
    names.dedup();
    names
}

/// The recorded classes' answers to the functions their configs were asked
/// (before the engine's own behaviours: those keep their place); camelot's.
pub fn recorded_behaviours(flavor: &str) -> Vec<(&'static str, NativeFn)> {
    if flavor != "camelot" {
        return Vec::new();
    }
    recorded_names()
        .into_iter()
        .map(|name| (name, recorded as NativeFn))
        .collect()
}

/// The modelled functions (camelot's data).
pub fn behaviours(flavor: &str) -> Vec<(&'static str, NativeFn)> {
    if flavor != "camelot" {
        return Vec::new();
    }
    vec![
        ("C_Traits.GetNodeInfo", get_node_info),
        ("C_Traits.GetConditionInfo", get_condition_info),
        ("C_Traits.GetTreeCurrencyInfo", get_tree_currency_info),
        ("C_Traits.CanPurchaseRank", can_purchase_rank),
        ("C_Traits.CanRefundRank", can_refund_rank),
        ("C_Traits.ConfigHasStagedChanges", config_has_staged_changes),
        ("C_Traits.GetStagedChanges", get_staged_changes),
        ("C_Traits.PurchaseRank", purchase_rank),
        ("C_Traits.CommitConfig", commit_config),
        ("C_ClassTalents.CommitConfig", commit_class_config),
        ("C_Traits.RollbackConfig", rollback_config),
        ("C_Traits.GetConfigInfo", get_config_info),
        (
            "C_MajorFactions.GetCurrentRenownLevel",
            get_current_renown_level,
        ),
        (
            "C_MajorFactions.GetMajorFactionProgressionInfo",
            get_major_faction_progression_info,
        ),
        (
            "C_MajorFactions.GetTotalReputationForRenownLevel",
            get_total_reputation_for_renown_level,
        ),
        (
            "C_MajorFactions.GetRenownRewardsForLevel",
            get_renown_rewards_for_level,
        ),
    ]
}

/// The fields two answers differ in: ours, theirs.
fn differences(ours: &Option<Rv>, theirs: &Option<Rv>) -> String {
    let (Some(Rv::Table(a)), Some(Rv::Table(b))) = (ours, theirs) else {
        return format!("{ours:?} / {theirs:?}");
    };
    let mut out = Vec::new();
    for (k, v) in a {
        match b.iter().find(|(kb, _)| kb == k) {
            Some((_, w)) if w == v => {}
            Some((_, w)) => out.push(format!("{k}: {v:?} / {w:?}")),
            None => out.push(format!("{k}: {v:?} / -")),
        }
    }
    for (k, w) in b {
        if !a.iter().any(|(ka, _)| ka == k) {
            out.push(format!("{k}: - / {w:?}"));
        }
    }
    out.join("; ")
}

/// The recorded answers again from the client's trees and the recorded
/// ranks (decision T435): the number compared and those that differ.
/// Every node, entry, definition, cost, condition and tree currency of
/// every recorded class.
pub fn compare_with_recording(client: Arc<TraitTables>) -> (usize, Vec<String>) {
    let state = Traits::default();
    let mut checked = 0;
    let mut wrong = Vec::new();
    let mut calls: Vec<_> = data().iter().collect();
    calls.sort_unstable();
    for ((class, call), answer) in calls {
        let m = Model::new(class, &state).with_client(Some(client.clone()));
        let args = call_ints(call);
        let name = call.split('(').next().unwrap_or_default();
        let ours = match name {
            "C_Traits.GetNodeInfo" => m.node(args[0], args[1]),
            "C_Traits.GetConditionInfo" => m.condition(args[0], args[1]),
            "C_Traits.GetTreeCurrencyInfo" => {
                // Only the configs the model knows the trees of.
                if !m.trees(args[0]).contains(&args[1]) {
                    continue;
                }
                m.tree_currency(args[0], args[1], call.ends_with("true)"))
            }
            _ => match m.client_answer(name, call) {
                Some(rv) => rv,
                None => continue,
            },
        };
        let theirs = Rv::answer(answer).into_iter().next();
        let (ours, theirs) = (ours.map(canonical), theirs.map(canonical));
        if ours != theirs {
            wrong.push(format!("{class} {call}: {}", differences(&ours, &theirs)));
        }
        checked += 1;
    }
    (checked, wrong)
}

/// An answer with its tables' fields in one order (a Lua table has none):
/// numbers' keys by number, the others by name.
pub(crate) fn canonical(rv: Rv) -> Rv {
    let Rv::Table(fields) = rv else {
        return rv;
    };
    let mut fields: Vec<(String, Rv)> =
        fields.into_iter().map(|(k, v)| (k, canonical(v))).collect();
    fields.sort_by(|a, b| match (a.0.parse::<f64>(), b.0.parse::<f64>()) {
        (Ok(x), Ok(y)) => x.total_cmp(&y),
        _ => a.0.cmp(&b.0),
    });
    Rv::Table(fields)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A fixture's entries (`["call"] = "answer",`, Lua's escapes).
    fn fixture(text: &str) -> Vec<(String, String)> {
        fn lua(s: &str) -> String {
            let mut out = Vec::new();
            let b = s.as_bytes();
            let mut i = 0;
            while i < b.len() {
                if b[i] == b'\\' && i + 1 < b.len() {
                    i += 1;
                    match b[i] {
                        b'n' => out.push(b'\n'),
                        b'r' => out.push(b'\r'),
                        b'0'..=b'9' => {
                            let end = (i + 3).min(b.len());
                            let digits: String = s[i..end]
                                .chars()
                                .take_while(|c| c.is_ascii_digit())
                                .collect();
                            out.push(digits.parse::<u8>().unwrap_or(b'?'));
                            i += digits.len() - 1;
                        }
                        c => out.push(c),
                    }
                } else {
                    out.push(b[i]);
                }
                i += 1;
            }
            String::from_utf8_lossy(&out).into_owned()
        }
        text.lines()
            .filter_map(|l| {
                let l = l.trim().strip_prefix("[\"")?.strip_suffix("\",")?;
                let (k, v) = l.split_once("\"] = \"")?;
                Some((lua(k), lua(v)))
            })
            .collect()
    }

    /// The client's trees (decision T435): a hand-made tree in place of the
    /// recorded druid's (config 22147, tree 1089), two tabs (node groups 1
    /// and 2), a condition of 2 points spent in the first tab.
    #[test]
    fn the_client_trees_give_the_structure() {
        use crate::clientdata::{
            TraitCondition, TraitDefinition, TraitEntry, TraitNode, TraitTree,
        };
        let mut t = TraitTables::default();
        t.trees.insert(
            1089,
            TraitTree {
                flags: CANNOT_REFUND,
                nodes: vec![1001, 1002, 1003],
                costs: vec![(3820, 1)],
            },
        );
        for (id, group, conditions) in [(1001, 1, vec![]), (1002, 1, vec![900]), (1003, 2, vec![])]
        {
            t.nodes.insert(
                id,
                TraitNode {
                    tree: 1089,
                    entries: vec![id + 1000],
                    groups: vec![group],
                    conditions,
                    ..Default::default()
                },
            );
            t.entries.insert(
                id + 1000,
                TraitEntry {
                    definition: id + 2000,
                    max_ranks: 5,
                    ..Default::default()
                },
            );
            t.definitions.insert(
                id + 2000,
                TraitDefinition {
                    spell: id + 3000,
                    ..Default::default()
                },
            );
        }
        t.groups.insert(1, vec![1001, 1002]);
        t.groups.insert(2, vec![1003]);
        t.conditions.insert(
            900,
            TraitCondition {
                tree: 1089,
                group: 1,
                currency: 3820,
                spent: 2,
                tooltip: Some("Spend %d more |4point:points; in %s".into()),
                ..Default::default()
            },
        );
        let client = Arc::new(t);
        let mut state = Traits::default();
        let available = |state: &Traits| {
            let m = Model::new("DRUID-22147", state).with_client(Some(client.clone()));
            let node = m.node(22147, 1002).unwrap();
            (node.flag("isAvailable"), m.condition(22147, 900).unwrap())
        };
        // Points in the other tab do not count.
        state.committed.insert((22147, 1003), 5);
        let (open, cond) = available(&state);
        assert!(!open);
        assert_eq!(cond.num("spentAmountRequired"), Some(2.0));
        assert_eq!(
            cond.get("tooltipText"),
            Some(&Rv::Str(
                "|cffff2020Spend 2 more |4point:points; in |r".into()
            ))
        );
        state.committed.insert((22147, 1001), 2);
        let (open, cond) = available(&state);
        assert!(open);
        assert!(cond.flag("isMet"));
        assert_eq!(cond.get("tooltipText"), None);

        let m = Model::new("DRUID-22147", &state).with_client(Some(client.clone()));
        assert_eq!(m.tree_nodes(1089), [1001, 1002, 1003]);
        assert_eq!(m.node_spells(22147, 1001), [4001]);
        let node = m.node(22147, 1001).unwrap();
        assert_eq!(node.num("ranksPurchased"), Some(2.0));
        assert_eq!(node.num("maxRanks"), Some(5.0));
        // A recorded node the build lacks: none.
        assert_eq!(m.node(22147, 104925), None);
        let entry = m
            .client_answer(
                "C_Traits.GetEntryInfo",
                "C_Traits.GetEntryInfo(22147, 2001)",
            )
            .flatten()
            .unwrap();
        assert_eq!(entry.num("definitionID"), Some(3001.0));
        let def = m
            .client_answer(
                "C_Traits.GetDefinitionInfo",
                "C_Traits.GetDefinitionInfo(3001)",
            )
            .flatten()
            .unwrap();
        assert_eq!(def, fields(vec![("spellID", num(4001))]));
        let cost = m
            .client_answer("C_Traits.GetNodeCost", "C_Traits.GetNodeCost(22147, 1001)")
            .flatten()
            .unwrap();
        assert_eq!(cost, costs(&[(3820, 1)]));
    }

    /// Another class's node asked through the class config answers as with
    /// nothing of its tree bought, never to buy: the druid's recording of
    /// every other class's tree, worked out from their owners'.
    #[test]
    fn another_class_s_nodes_follow_from_their_owners() {
        let state = Traits::default();
        let m = Model::new("DRUID-2839888", &state);
        let recorded = fixture(include_str!(
            "../tests/client/forever-camelot-traits-druid-2839888.lua"
        ));
        let mut checked = 0;
        let mut wrong = Vec::new();
        for (call, answer) in &recorded {
            let Some(inner) = call
                .strip_prefix("C_Traits.GetNodeInfo(")
                .and_then(|r| r.strip_suffix(')'))
            else {
                continue;
            };
            let args: Vec<i64> = inner.split(", ").filter_map(|a| a.parse().ok()).collect();
            // Only nodes of trees the druid's configs do not hold.
            if m.recorded_node(args[0], args[1]).is_some() {
                continue;
            }
            let ours = m.node(args[0], args[1]);
            let theirs = Rv::answer(answer).into_iter().next();
            if ours != theirs {
                wrong.push(format!("{call}: {}", differences(&ours, &theirs)));
            }
            checked += 1;
        }
        assert!(checked > 100, "{checked}");
        assert!(
            wrong.is_empty(),
            "{} of {checked}:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }

    /// The recorded configs' answers again from the ranks: every node,
    /// condition and tree currency of every recorded class as recorded.
    #[test]
    fn the_recorded_answers_follow_from_the_ranks() {
        let state = Traits::default();
        let mut checked = 0;
        let mut wrong = Vec::new();
        for ((class, call), answer) in data() {
            let m = Model::new(class, &state);
            let args = |prefix: &str| -> Option<Vec<i64>> {
                let inner = call.strip_prefix(prefix)?.strip_suffix(')')?;
                inner.split(", ").map(|a| a.parse().ok()).collect()
            };
            let ours = if let Some(a) = args("C_Traits.GetNodeInfo(") {
                m.node(a[0], a[1])
            } else if let Some(a) = args("C_Traits.GetConditionInfo(") {
                m.condition(a[0], a[1])
            } else if let Some(rest) = call.strip_prefix("C_Traits.GetTreeCurrencyInfo(") {
                let parts: Vec<&str> = rest.trim_end_matches(')').split(", ").collect();
                let (Ok(config), Ok(tree)) = (parts[0].parse(), parts[1].parse()) else {
                    continue;
                };
                // Only the configs the model knows the trees of.
                if !m.trees(config).contains(&tree) {
                    continue;
                }
                m.tree_currency(config, tree, parts[2] == "true")
            } else {
                continue;
            };
            let theirs = Rv::answer(answer).into_iter().next();
            if ours != theirs {
                wrong.push(format!("{class} {call}: {}", differences(&ours, &theirs)));
            }
            checked += 1;
        }
        assert!(checked > 0);
        assert!(
            wrong.is_empty(),
            "{} of {checked}:\n{}",
            wrong.len(),
            wrong.join("\n")
        );
    }
}
