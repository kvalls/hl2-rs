//! ai_goal_actbusy_queue: an actbusy goal whose hint nodes form a line (Source SDK
//! hl2/ai_behavior_actbusy.cpp CAI_ActBusyQueueGoal; retail server.dll carries the same inputs,
//! outputs and thinks). d1_trainstation_01's customs queue: four citizens forced onto the first
//! nodes, a logic_timer sending MoveQueueUp every 5 s, OnNPCLeftQueue counting who left.
//!
//! The front node's NPC leaves on MoveQueueUp and walks to the exit target; each NPC that settles
//! on its node lets the queue move up 0.3-0.5 s later, when the nearest NPC behind each empty
//! node walks forward one gap (the others wait for the next settle, so the line ripples); every
//! 5 s an empty front node moves the queue up too. Not in: the player blocking nodes
//! (PlayerStartedBlocking/PlayerStoppedBlocking), the hint types and disabled nodes that steer
//! newcomers to the back, OnQueueMoved's count value, OnNPCStartedLeavingQueue waiting for an
//! exit animation (ours has none: it fires with MoveQueueUp).
use super::AiRuntime;
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
    /// NPCs seen busy on their node (a new one moves the queue up).
    settled: BTreeSet<usize>,
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
        Self {
            nodes,
            exit,
            must_reach_front: e.get("mustreachfront").is_some_and(|v| v.trim() == "1"),
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
    pub(super) fn queue_input(&mut self, scene: &mut Scene, goal: usize, input: &str) -> bool {
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
                *self
                    .diagnostics
                    .unknown_goals
                    .entry(format!("actbusy queue {input}"))
                    .or_default() += 1;
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
            let count = holders
                .iter()
                .rposition(Option::is_some)
                .map_or(0, |i| i + 1);
            if count != queue.count {
                queue.count = count;
                scene.fire(goal, "OnQueueMoved", goal);
            }
            if queue.move_up_at.is_some_and(|at| at <= now) {
                queue.move_up_at = None;
                self.move_queue_up(goal, &holders, now);
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
        let nodes = self.actbusy_ranges.queues[&goal].nodes.clone();
        let range = self.actbusy_ranges.get(&goal).copied().unwrap_or(2048.);
        let mut moved = BTreeSet::new();
        for empty in 0..holders.len().saturating_sub(1) {
            if holders[empty].is_some() || nodes[empty].is_none() {
                continue;
            }
            let behind = holders[empty + 1..]
                .iter()
                .flatten()
                .map(|h| h.0)
                .find(|id| !moved.contains(id));
            let Some(npc) = behind else {
                break;
            };
            moved.insert(npc);
            let Some(actor) = self.actors.get_mut(&npc) else {
                continue;
            };
            if let Some(busy) = actor.class.actbusy_mut() {
                busy.force(Some(goal), range, None, false, &mut actor.npc, now);
                busy.forced_hint = nodes[empty];
                // A busy NPC's looping busy schedule ignores the provocation: select anew.
                actor.npc.schedule = None;
            }
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
