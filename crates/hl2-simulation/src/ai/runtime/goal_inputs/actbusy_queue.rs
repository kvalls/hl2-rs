//! ai_goal_actbusy_queue: an actbusy goal whose hint nodes form a line (Source SDK
//! hl2/ai_behavior_actbusy.cpp CAI_ActBusyQueueGoal; retail server.dll carries the same inputs,
//! outputs and thinks). d1_trainstation_01's customs queue: four citizens forced onto the first
//! nodes, a logic_timer sending MoveQueueUp every 5 s, OnNPCLeftQueue counting who left.
//!
//! The front node's NPC leaves on MoveQueueUp and walks to the exit target; each NPC that settles
//! on its node lets the queue move up 0.3-0.5 s later, when the nearest NPC behind each empty
//! node walks forward one gap (the others wait for the next settle, so the line ripples); every
//! 5 s an empty front node moves the queue up too. NPCs outside the line join at its back
//! (steer_newcomers). The map's client-only triggers on the nodes send PlayerStartedBlocking and
//! PlayerStoppedBlocking: a node the player stands on counts as held, its NPC and those it
//! displaces move back, and the line never moves up past the player (player_blocking). Not in:
//! OnQueueMoved's count value, OnNPCStartedLeavingQueue waiting for an exit animation (ours has
//! none: it fires with MoveQueueUp).
use super::AiRuntime;
use crate::ai::actbusy::{HintCandidate, HINT_WORLD_WORK_POSITION};
use crate::entities::Scene;
use modkit_core::World;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet};

/// MAX_QUEUE_NODES: node01 .. node20.
const MAX_NODES: usize = 20;
/// QueueThink's period (seconds).
const CHECK_PERIOD: f64 = 5.;

/// Each actbusy goal's search range and each queue's state.
#[derive(Default, Serialize)]
pub(in crate::ai::runtime) struct ActBusyGoals {
    ranges: BTreeMap<usize, f32>,
    queues: BTreeMap<usize, Queue>,
}

#[derive(Default, Serialize)]
struct Queue {
    /// The hint node entities in line order (None: an empty slot).
    nodes: Vec<Option<usize>>,
    exit: Option<usize>,
    /// mustreachfront: MoveQueueUp skips NPCs still walking to their node.
    must_reach_front: bool,
    /// Every named node is an info_node_hint and the exit exists (else the SDK removes it).
    valid: bool,
    active: bool,
    exiting: Option<usize>,
    move_up_at: Option<f64>,
    next_check: f64,
    count: usize,
    /// The count the nodes' hint types were last set for (steer_newcomers).
    steered: Option<usize>,
    /// NPCs seen busy on their node (a new one moves the queue up).
    settled: BTreeSet<usize>,
    /// m_bPlayerBlockedNodes: the nodes whose trigger the player stands in.
    player_blocked: Vec<bool>,
}

/// RecalculateQueueCount's count: one past the last node an NPC holds or the player blocks.
fn line_count(holders: &[Option<(usize, bool)>], blocked: &[bool]) -> usize {
    let held =
        |i: usize| holders.get(i).is_some_and(Option::is_some) || blocked.get(i) == Some(&true);
    (0..holders.len().max(blocked.len()))
        .rev()
        .find(|&i| held(i))
        .map_or(0, |i| i + 1)
}

/// PushNPCBackInQueue's moves as (NPC, node index): the NPC holding `node` takes the next node
/// back the player does not block, displacing that node's NPC in turn, until a free node.
fn push_back_moves(
    holders: &[Option<(usize, bool)>],
    blocked: &[bool],
    node: usize,
) -> Vec<(usize, usize)> {
    let holder = |i: usize| holders.get(i).copied().flatten().map(|h| h.0);
    let mut moves = Vec::new();
    let (mut npc, mut from) = (holder(node), node);
    while let Some(id) = npc {
        let Some(next) = (from + 1..MAX_NODES).find(|&i| blocked.get(i) != Some(&true)) else {
            break;
        };
        moves.push((id, next));
        npc = holder(next).filter(|&d| d != id);
        from = next;
    }
    moves
}

/// RecalculateQueueCount: the nodes more than one past the last held node are disabled; the
/// node just behind the line (the entry) is the only actbusy hint (HINT_WORLD_WORK_POSITION)
/// and the held ones are HINT_NONE, so an NPC outside the queue joins at its back instead of
/// walking to a free node inside it. Forced hints (MoveQueueUp, ForceNPCToActBusy) ignore both.
fn steer_newcomers(hints: &mut [HintCandidate], nodes: &[Option<usize>], count: usize) {
    for (i, node) in nodes.iter().enumerate() {
        let Some(hint) = node.and_then(|n| hints.iter_mut().find(|h| h.entity == n)) else {
            continue;
        };
        hint.disabled = i > count;
        hint.hint_type = if i == count {
            HINT_WORLD_WORK_POSITION
        } else {
            0
        };
    }
}

fn named(world: &World, name: &str) -> Option<usize> {
    world.entities.iter().position(|e| {
        e.get("targetname")
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
    })
}

impl ActBusyGoals {
    /// Every ai_goal_actbusy and ai_goal_actbusy_queue with its busysearchrange.
    pub(in crate::ai::runtime) fn new(world: &World) -> Self {
        let mut goals = Self::default();
        for (id, e) in world.entities.iter().enumerate() {
            let class = e.class();
            if class != "ai_goal_actbusy" && class != "ai_goal_actbusy_queue" {
                continue;
            }
            let range = e.get("busysearchrange").and_then(|v| v.trim().parse().ok());
            goals.ranges.insert(id, range.unwrap_or(2048.));
            if class == "ai_goal_actbusy_queue" {
                goals.queues.insert(id, Queue::new(world, e));
            }
        }
        goals
    }
    pub(in crate::ai::runtime) fn get(&self, goal: &usize) -> Option<&f32> {
        self.ranges.get(goal)
    }
    pub(in crate::ai::runtime) fn insert(&mut self, goal: usize, range: f32) {
        self.ranges.insert(goal, range);
    }
}

impl Queue {
    fn new(world: &World, e: &modkit_core::Entity) -> Self {
        let mut valid = true;
        let mut nodes = Vec::new();
        for i in 1..=MAX_NODES {
            let Some(name) = e
                .get(&format!("node{i:02}"))
                .filter(|n| !n.trim().is_empty())
            else {
                nodes.push(None);
                continue;
            };
            let node = named(world, name.trim())
                .filter(|&n| world.entities[n].class() == "info_node_hint");
            valid &= node.is_some();
            nodes.push(node);
        }
        while nodes.last() == Some(&None) {
            nodes.pop();
        }
        let exit = e.get("node_exit").and_then(|n| named(world, n.trim()));
        let player_blocked = vec![false; nodes.len()];
        Self {
            nodes,
            player_blocked,
            exit,
            must_reach_front: e.get("mustreachfront").is_some_and(|v| v.trim() == "1"),
            steered: None,
            valid: valid && exit.is_some(),
            ..Self::default()
        }
    }
}

impl AiRuntime {
    /// The NPC holding each node (walking to it or busy on it): its busy is enabled, so the
    /// leaving NPC (disabled) frees its node.
    fn queue_holders(&mut self, goal: usize) -> Vec<Option<(usize, bool)>> {
        let Some(queue) = self.actbusy_ranges.queues.get(&goal) else {
            return Vec::new();
        };
        let (nodes, exiting) = (queue.nodes.clone(), queue.exiting);
        let mut holders = vec![None; nodes.len()];
        for (&id, actor) in &mut self.actors {
            let Some(busy) = actor.class.actbusy_mut() else {
                continue;
            };
            if busy.goal != Some(goal) || !busy.enabled || Some(id) == exiting {
                continue;
            }
            let held = busy.forced_hint.or(busy.hint.as_ref().map(|h| h.entity));
            if let Some(slot) = nodes.iter().position(|n| n.is_some() && *n == held) {
                holders[slot] = Some((id, busy.busy));
            }
        }
        holders
    }

    /// Queue-only inputs (true: handled); Activate also starts the queue and then reaches the
    /// actbusy goal's own Activate.
    pub(super) fn queue_input(
        &mut self,
        scene: &mut Scene,
        goal: usize,
        input: &str,
        parameter: &str,
    ) -> bool {
        let now = scene.time;
        match input {
            "activate" => {
                if let Some(queue) = self.actbusy_ranges.queues.get_mut(&goal) {
                    if queue.valid && !queue.active {
                        queue.active = true;
                        queue.next_check = now + CHECK_PERIOD;
                    }
                }
                false
            }
            "movequeueup" => {
                self.move_front_out(scene, goal);
                true
            }
            "playerstartedblocking" | "playerstoppedblocking" => {
                // The value is the node's number (1-based).
                let node = parameter.trim().parse::<usize>().ok();
                if let Some(node) = node
                    .and_then(|n| n.checked_sub(1))
                    .filter(|&n| n < MAX_NODES)
                {
                    self.player_blocking(goal, node, input == "playerstartedblocking", now);
                }
                true
            }
            _ => false,
        }
    }

    /// InputMoveQueueUp: the front-most NPC (one still walking to its node too, unless the
    /// queue must reach the front) stops busying and leaves.
    fn move_front_out(&mut self, scene: &mut Scene, goal: usize) {
        let must_reach_front = self
            .actbusy_ranges
            .queues
            .get(&goal)
            .is_some_and(|q| q.must_reach_front);
        let front = self
            .queue_holders(goal)
            .into_iter()
            .flatten()
            .find(|&(_, busy)| busy || !must_reach_front);
        let Some((npc, _)) = front else {
            return;
        };
        let now = scene.time;
        if let Some(actor) = self.actors.get_mut(&npc) {
            if let Some(busy) = actor.class.actbusy_mut() {
                busy.disable(&mut actor.npc, now);
            }
        }
        if let Some(queue) = self.actbusy_ranges.queues.get_mut(&goal) {
            queue.exiting = Some(npc);
        }
        scene.fire(goal, "OnNPCStartedLeavingQueue", npc);
    }

    /// Each active queue's tick: settled NPCs and the 5 s check schedule a move up, the leaving
    /// NPC walks to the exit, a due move up pulls the NPCs behind the gaps forward.
    pub(super) fn think_actbusy_queues(&mut self, world: &World, scene: &mut Scene) {
        let goals: Vec<usize> = (self.actbusy_ranges.queues.iter())
            .filter(|(_, q)| q.active)
            .map(|(&id, _)| id)
            .collect();
        for goal in goals {
            self.queue_leaver(world, scene, goal);
            let holders = self.queue_holders(goal);
            let now = scene.time;
            let jitter = 0.3 + 0.2 * f64::from(self.random());
            let queue = self.actbusy_ranges.queues.get_mut(&goal).expect("queue");
            let busy: BTreeSet<usize> = holders
                .iter()
                .flatten()
                .filter(|h| h.1)
                .map(|h| h.0)
                .collect();
            let settled_new = busy.difference(&queue.settled).next().is_some();
            queue.settled = busy;
            let empty_front = holders.first().is_some_and(Option::is_none);
            if now >= queue.next_check {
                queue.next_check = now + CHECK_PERIOD;
                if empty_front && queue.move_up_at.is_none() {
                    queue.move_up_at = Some(now + jitter);
                }
            }
            if settled_new && queue.move_up_at.is_none() {
                queue.move_up_at = Some(now + jitter);
            }
            let count = line_count(&holders, &queue.player_blocked);
            if count != queue.count {
                queue.count = count;
                // COutputInt m_OnQueueMoved: the new count is the output's value.
                scene.fire_value(goal, "OnQueueMoved", count as f32, goal);
            }
            let steer = (queue.steered != Some(count)).then(|| queue.nodes.clone());
            queue.steered = Some(count);
            if queue.move_up_at.is_some_and(|at| at <= now) {
                queue.move_up_at = None;
                self.move_queue_up(goal, &holders, now);
            }
            if let Some(nodes) = steer {
                steer_newcomers(&mut self.busy_hints, &nodes, count);
            }
        }
    }

    /// The leaving NPC, once its busy has ended, walks to the exit target (SCHED_IDLE_WALK in
    /// the SDK) and OnNPCLeftQueue fires; the line then moves up.
    fn queue_leaver(&mut self, world: &World, scene: &mut Scene, goal: usize) {
        let Some(queue) = self.actbusy_ranges.queues.get(&goal) else {
            return;
        };
        let (Some(npc), exit) = (queue.exiting, queue.exit) else {
            return;
        };
        let Some(actor) = self.actors.get_mut(&npc) else {
            self.actbusy_ranges
                .queues
                .get_mut(&goal)
                .expect("queue")
                .exiting = None;
            return;
        };
        if actor
            .class
            .actbusy_mut()
            .is_some_and(|b| b.busy || b.moving_to_busy)
        {
            return;
        }
        // Its node is free for the line: a disabled busy keeps its hint (and the host's lock).
        if let Some(busy) = actor.class.actbusy_mut() {
            if let Some(hint) = busy.hint.take() {
                busy.released.push(hint.entity);
            }
            busy.forced_hint = None;
        }
        if let Some(exit) = exit {
            let at = if scene.states.get(exit).is_some_and(|s| !s.killed) {
                scene.states[exit].origin
            } else {
                world.entities[exit].origin()
            };
            *actor.class.script_goal_mut() = Some((at, false));
            actor.npc.force_schedule("SCHED_AISCRIPTED_MOVE_TO_GOAL");
        }
        scene.fire(goal, "OnNPCLeftQueue", npc);
        let now = scene.time;
        let jitter = 0.3 + 0.2 * f64::from(self.random());
        let queue = self.actbusy_ranges.queues.get_mut(&goal).expect("queue");
        queue.exiting = None;
        queue.move_up_at.get_or_insert(now + jitter);
    }

    /// MoveQueueUpThink: for each empty node from the front, the nearest NPC behind it (in this
    /// think's snapshot) is forced onto it; the node it leaves counts as held until the next
    /// think, so each NPC moves one gap per think.
    fn move_queue_up(&mut self, goal: usize, holders: &[Option<(usize, bool)>], now: f64) {
        let queue = &self.actbusy_ranges.queues[&goal];
        let (nodes, blocked) = (queue.nodes.clone(), queue.player_blocked.clone());
        let is_blocked = |i: usize| blocked.get(i) == Some(&true);
        let mut moved = BTreeSet::new();
        for empty in 0..holders.len().saturating_sub(1) {
            if holders[empty].is_some() || nodes[empty].is_none() || is_blocked(empty) {
                continue;
            }
            // Nobody comes forward from the far side of the player.
            let behind = (empty + 1..holders.len())
                .take_while(|&i| !is_blocked(i))
                .filter_map(|i| holders[i])
                .map(|h| h.0)
                .find(|id| !moved.contains(id));
            let Some(npc) = behind else {
                continue;
            };
            moved.insert(npc);
            self.force_onto_node(goal, npc, nodes[empty], now);
        }
    }

    /// ForceActBusy(goal, node): the NPC walks to `node` (None: a hint the goal offers).
    fn force_onto_node(&mut self, goal: usize, npc: usize, node: Option<usize>, now: f64) {
        let range = self.actbusy_ranges.get(&goal).copied().unwrap_or(2048.);
        let Some(actor) = self.actors.get_mut(&npc) else {
            return;
        };
        if let Some(busy) = actor.class.actbusy_mut() {
            busy.force(Some(goal), range, None, false, &mut actor.npc, now);
            busy.forced_hint = node;
            // A busy NPC's looping busy schedule ignores the provocation: select anew.
            actor.npc.schedule = None;
        }
    }

    /// InputPlayerStartedBlocking / InputPlayerStoppedBlocking: a blocked node counts as held
    /// (line_count); blocking pushes its NPC back (PushNPCBackInQueue), unblocking moves the
    /// line up 0.3-0.5 s later (MoveQueueUp).
    fn player_blocking(&mut self, goal: usize, node: usize, started: bool, now: f64) {
        let jitter = 0.3 + 0.2 * f64::from(self.random());
        let Some(queue) = self.actbusy_ranges.queues.get_mut(&goal) else {
            return;
        };
        if queue.player_blocked.len() <= node {
            queue.player_blocked.resize(node + 1, false);
        }
        queue.player_blocked[node] = started;
        if !started {
            queue.move_up_at.get_or_insert(now + jitter);
            return;
        }
        let (nodes, blocked) = (queue.nodes.clone(), queue.player_blocked.clone());
        let holders = self.queue_holders(goal);
        for (npc, next) in push_back_moves(&holders, &blocked, node) {
            self.force_onto_node(goal, npc, nodes.get(next).copied().flatten(), now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use modkit_core::Entity;

    fn entity(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
        }
    }

    /// RecalculateQueueCount with two NPCs in line: only the third node takes newcomers, the
    /// fourth is disabled, the held ones are no actbusy hints.
    #[test]
    fn only_the_node_behind_the_line_takes_newcomers() {
        let mut hints: Vec<HintCandidate> = (10..14)
            .map(|entity| HintCandidate {
                entity,
                origin: glam::Vec3::ZERO,
                yaw: 0.,
                hint_type: HINT_WORLD_WORK_POSITION,
                group: String::new(),
                activity: "LineIdle01".into(),
                disabled: false,
                min_state: 1,
                max_state: 3,
            })
            .collect();
        let nodes = [Some(10), Some(11), Some(12), Some(13)];
        steer_newcomers(&mut hints, &nodes, 2);
        let types: Vec<_> = hints.iter().map(|h| (h.hint_type, h.disabled)).collect();
        assert_eq!(types, [(0, false), (0, false), (12, false), (0, true)]);
    }

    /// The player standing on node 2 of a line of three: the node counts as held, its NPC moves
    /// to node 3 and node 3's to node 4 (a free node ends the chain); with nodes 2 and 3 both
    /// blocked, node 2's NPC skips to node 4. A blocked node past the line extends the count.
    #[test]
    fn a_node_the_player_blocks_pushes_its_npcs_back() {
        let holders = [Some((7, true)), Some((8, true)), Some((9, false))];
        let blocked = [false, true, false];
        assert_eq!(push_back_moves(&holders, &blocked, 1), [(8, 2), (9, 3)]);
        assert_eq!(line_count(&holders, &blocked), 3);
        let both = [false, true, true];
        assert_eq!(push_back_moves(&holders, &both, 1), [(8, 3)]);
        assert!(push_back_moves(&holders, &blocked, 3).is_empty());
        let past = [false, false, false, false, true];
        assert_eq!(line_count(&holders, &past), 5);
        assert_eq!(line_count(&[None, None], &[]), 0);
    }

    /// d1_trainstation_01's customs queue: nodes in node01.. order, the exit target, and a queue
    /// whose node is not a hint (or whose exit is missing) never activates.
    #[test]
    fn a_queue_resolves_its_nodes_in_order_and_needs_hints_and_an_exit() {
        let queue = |node02: &str, exit: &str| World {
            entities: vec![
                entity(&[
                    ("classname", "ai_goal_actbusy_queue"),
                    ("node01", "hint_1"),
                    ("node02", node02),
                    ("node_exit", exit),
                    ("busysearchrange", "512"),
                ]),
                entity(&[("classname", "info_node_hint"), ("targetname", "hint_1")]),
                entity(&[("classname", "info_node_hint"), ("targetname", "hint_2")]),
                entity(&[("classname", "info_target"), ("targetname", "out")]),
            ],
            ..Default::default()
        };
        let goals = ActBusyGoals::new(&queue("hint_2", "out"));
        assert_eq!(goals.get(&0), Some(&512.));
        let q = &goals.queues[&0];
        assert_eq!(q.nodes, vec![Some(1), Some(2)]);
        assert_eq!(q.exit, Some(3));
        assert!(q.valid);
        assert!(!ActBusyGoals::new(&queue("out", "out")).queues[&0].valid);
        assert!(!ActBusyGoals::new(&queue("hint_2", "nowhere")).queues[&0].valid);
    }
}
