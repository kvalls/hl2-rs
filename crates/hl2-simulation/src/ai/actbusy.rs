//! ai_goal_actbusy and CAI_ActBusyBehavior (hl2/ai_behavior_actbusy.cpp), the
//! non-combat subset: an enabled (Activate) or forced (ForceNPCToActBusy) NPC finds a
//! HINT_WORLD_WORK_POSITION hint node of its hint group within the busy search range
//! (random, or the nearest), walks there, faces along the hint, plays the hint's busy
//! activity from scripts/actbusy.txt for U(min_time, max_time) seconds (forever without a
//! max time) and then searches again. SDK behavior; entry/exit animations, sounds, see
//! entities, queues, combat actbusies, leaving and teleports are not reconstructed.
use super::conditions::COND_PROVOKED;
use super::schedule::{
    AiNpc, Behavior, Context, Goal, Motor, Schedule, Schedules, TaskArg, TaskStatus,
};
use glam::Vec3;
use serde::Serialize;

/// ai_actbusy_search_time.
pub const SEARCH_TIME: f64 = 10.;
/// ai_hint.h HINT_WORLD_WORK_POSITION.
pub const HINT_WORLD_WORK_POSITION: i32 = 12;

const SCHEDULES: &[(&str, &str)] = &[
    (
        "SCHED_ACTBUSY_START_BUSYING",
        "Tasks TASK_SET_TOLERANCE_DISTANCE 4 TASK_ACTBUSY_GET_PATH_TO_ACTBUSY 0 \
         TASK_ACTBUSY_WALK_PATH_TO_BUSY 0 TASK_WAIT_FOR_MOVEMENT 0 TASK_STOP_MOVING 0 \
         TASK_FACE_HINTNODE 0 TASK_ACTBUSY_PLAY_ENTRY 0 TASK_SET_SCHEDULE SCHEDULE:SCHED_ACTBUSY_BUSY \
         Interrupts",
    ),
    (
        "SCHED_ACTBUSY_BUSY",
        "Tasks TASK_ACTBUSY_PLAY_BUSY_ANIM 0 Interrupts COND_PROVOKED",
    ),
    (
        "SCHED_ACTBUSY_STOP_BUSYING",
        "Tasks TASK_ACTBUSY_VERIFY_EXIT 0 TASK_ACTBUSY_PLAY_EXIT 0 TASK_WAIT 0.1 Interrupts",
    ),
];
pub fn add_schedules(schedules: &mut Schedules) {
    for (name, text) in SCHEDULES {
        let schedule = Schedule::parse(name, text).expect("actbusy schedule");
        schedules.0.insert((*name).into(), schedule);
    }
}

/// A busy animation from scripts/actbusy.txt (busyanim_t, activity form).
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct BusyAnim {
    pub name: String,
    pub activity: String,
    pub min_time: f32,
    pub max_time: f32,
}
/// CActBusyAnimData::ParseAnimDataFile, the entries whose busy animation is an activity
/// ("busy_anim"); sequence-only entries are skipped.
pub fn parse_busy_anims(text: &str) -> Vec<BusyAnim> {
    let Ok(root) = source_assets::keyvalues::parse(text) else {
        return Vec::new();
    };
    let mut anims = Vec::new();
    for block in root.iter().flat_map(|r| r.children()) {
        let name = block.key.as_str();
        let get = |k: &str| {
            block
                .get(k)
                .and_then(|e| e.text())
                .map(str::trim)
                .filter(|v| !v.is_empty())
        };
        let Some(activity) = get("busy_anim") else {
            continue;
        };
        let time = |k: &str| get(k).and_then(|v| v.parse().ok()).unwrap_or(0.);
        anims.push(BusyAnim {
            name: name.to_owned(),
            activity: activity.to_owned(),
            min_time: time("min_time"),
            max_time: time("max_time"),
        });
    }
    anims
}
/// FindBusyAnim: the entry whose busy activity, or name, is the hint's activity.
pub fn find_busy_anim<'a>(anims: &'a [BusyAnim], hint_activity: &str) -> Option<&'a BusyAnim> {
    anims
        .iter()
        .find(|a| a.activity.eq_ignore_ascii_case(hint_activity))
        .or_else(|| {
            anims
                .iter()
                .find(|a| a.name.eq_ignore_ascii_case(hint_activity))
        })
}

/// A locked hint node and its busy animation.
#[derive(Clone, Debug, Serialize)]
pub struct BusyHint {
    pub entity: usize,
    pub origin: Vec3,
    pub yaw: f32,
    pub anim: BusyAnim,
}

/// CAI_ActBusyBehavior state.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ActBusy {
    /// The ai_goal_actbusy that enabled it.
    pub goal: Option<usize>,
    pub enabled: bool,
    pub forced: bool,
    pub use_nearest: bool,
    /// The forced max time (None: NO_MAX_TIME, the animation's own times).
    pub forced_max_time: Option<f32>,
    pub busy: bool,
    pub moving_to_busy: bool,
    pub search_range: f32,
    pub next_search: f64,
    pub end_busy_at: f64,
    pub hint: Option<BusyHint>,
    /// A hint the host found for this think's selection (FindActBusyHintNode).
    #[serde(skip)]
    pub candidate: Option<BusyHint>,
    /// The hint to release (the host unlocks it).
    #[serde(skip)]
    pub released: Vec<usize>,
    /// ai_goal_actbusy outputs for the host to fire (OnNPCStartedBusy, ...).
    #[serde(skip)]
    pub outputs: Vec<&'static str>,
    /// The behavior selected the current schedule.
    pub running: bool,
}
impl ActBusy {
    /// Enable(goal, range, visible only).
    pub fn enable(&mut self, goal: Option<usize>, range: f32, now: f64) {
        self.release_hint();
        if goal.is_some() {
            self.goal = goal;
        }
        self.enabled = true;
        self.busy = false;
        self.moving_to_busy = false;
        self.next_search = now + SEARCH_TIME;
        self.end_busy_at = 0.;
        self.search_range = range;
    }
    /// ForceActBusy (no hint node, teleport, see entity or forced activity).
    pub fn force(
        &mut self,
        goal: Option<usize>,
        range: f32,
        max_time: Option<f32>,
        nearest: bool,
        npc: &mut AiNpc,
        now: f64,
    ) {
        self.enable(goal, range, now);
        self.forced = true;
        self.forced_max_time = max_time;
        self.use_nearest = nearest;
        npc.conditions.set(COND_PROVOKED);
    }
    /// Disable.
    pub fn disable(&mut self, npc: &mut AiNpc, now: f64) {
        if self.enabled {
            npc.conditions.set(COND_PROVOKED);
        }
        self.stop_busying(npc, now);
        self.enabled = false;
    }
    /// StopBusying.
    pub fn stop_busying(&mut self, npc: &mut AiNpc, now: f64) {
        if self.busy || self.moving_to_busy {
            npc.conditions.set(COND_PROVOKED);
        }
        self.end_busy_at = now;
        self.forced = false;
        self.use_nearest = false;
        self.moving_to_busy = false;
    }
    fn release_hint(&mut self) {
        if let Some(hint) = self.hint.take() {
            self.released.push(hint.entity);
        }
    }
    /// Whether the host should look for a hint this think.
    pub fn wants_hint(&self, now: f64) -> bool {
        self.enabled && !self.busy && self.hint.is_none() && (self.forced || self.next_search < now)
    }
    /// IsCurScheduleOverridable (non-combat): idle stand or no schedule.
    fn overridable(npc: &AiNpc) -> bool {
        matches!(npc.schedule.as_deref(), None | Some("SCHED_IDLE_STAND"))
    }
    /// CanSelectSchedule.
    pub fn can_select(&self, npc: &AiNpc, now: f64) -> bool {
        if self.busy || self.forced {
            return true;
        }
        self.enabled && Self::overridable(npc) && self.next_search < now
    }
    /// SelectSchedule. `base` is the NPC's own selection; actbusy acts only when it would
    /// idle (or when forced). None: keep the base schedule.
    pub fn select(&mut self, npc: &mut AiNpc, ctx: &Context, base: &str) -> Option<&'static str> {
        if !self.forced && base != "SCHED_IDLE_STAND" {
            if self.busy {
                return Some("SCHED_ACTBUSY_STOP_BUSYING");
            }
            return None;
        }
        if !self.enabled {
            return Some("SCHED_ACTBUSY_STOP_BUSYING");
        }
        if self.busy {
            if self.end_busy_at > 0. && ctx.now >= self.end_busy_at {
                self.stop_busying(npc, ctx.now);
                return Some("SCHED_ACTBUSY_STOP_BUSYING");
            }
            return Some("SCHED_ACTBUSY_BUSY");
        }
        // SelectScheduleWhileNotBusy.
        if self.forced || self.next_search < ctx.now {
            self.next_search = if self.forced {
                ctx.now + 2.
            } else {
                ctx.now + SEARCH_TIME * (1. + f64::from(ctx.random))
            };
            if self.hint.is_none() {
                self.hint = self.candidate.take();
            }
            if self.hint.is_some() {
                self.moving_to_busy = true;
                return Some("SCHED_ACTBUSY_START_BUSYING");
            }
        }
        None
    }
}
impl Behavior for ActBusy {
    fn start_task(
        &mut self,
        npc: &mut AiNpc,
        task: &str,
        arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        let _ = arg;
        Some(match task {
            "TASK_ACTBUSY_GET_PATH_TO_ACTBUSY" => match &self.hint {
                Some(hint) if motor.path_to(hint.origin, false) => TaskStatus::Complete,
                _ => TaskStatus::Failed,
            },
            "TASK_ACTBUSY_WALK_PATH_TO_BUSY" => {
                npc.activity = "ACT_WALK".into();
                motor.set_activity("ACT_WALK");
                TaskStatus::Complete
            }
            "TASK_FACE_HINTNODE" => match &self.hint {
                Some(hint) => {
                    if motor.face(Goal::Yaw(hint.yaw)) {
                        TaskStatus::Complete
                    } else {
                        npc.set_turn_activity(motor);
                        TaskStatus::Running
                    }
                }
                None => TaskStatus::Failed,
            },
            // No entry animations (the subset's hints have none).
            "TASK_ACTBUSY_PLAY_ENTRY" | "TASK_ACTBUSY_VERIFY_EXIT" => TaskStatus::Complete,
            "TASK_ACTBUSY_PLAY_BUSY_ANIM" => {
                if !self.enabled {
                    return Some(TaskStatus::Complete);
                }
                let Some(hint) = &self.hint else {
                    return Some(TaskStatus::Failed);
                };
                if self.moving_to_busy
                    && hint.origin.truncate().distance(ctx.origin.truncate()) > 16.
                {
                    self.end_busy_at = ctx.now;
                    return Some(TaskStatus::Failed);
                }
                self.moving_to_busy = false;
                if !self.busy {
                    self.busy = true;
                    self.outputs.push("OnNPCStartedBusy");
                    let (mut min, mut max) = (hint.anim.min_time, hint.anim.max_time);
                    if self.forced {
                        if let Some(forced) = self.forced_max_time {
                            max = forced;
                            if max > 0. && max < min {
                                min = max;
                            }
                        }
                    }
                    self.end_busy_at = if max <= 0. {
                        0.
                    } else {
                        ctx.now + f64::from(min + (max - min) * ctx.random)
                    };
                }
                npc.activity = hint.anim.activity.clone();
                motor.set_activity(&hint.anim.activity);
                self.forced = false;
                self.use_nearest = false;
                TaskStatus::Running
            }
            "TASK_ACTBUSY_PLAY_EXIT" => {
                if self.busy {
                    self.outputs.push("OnNPCFinishedBusy");
                }
                self.busy = false;
                self.release_hint();
                TaskStatus::Complete
            }
            _ => return None,
        })
    }
    fn run_task(
        &mut self,
        _npc: &mut AiNpc,
        task: &str,
        _arg: &TaskArg,
        ctx: &Context,
        motor: &mut dyn Motor,
    ) -> Option<TaskStatus> {
        match task {
            "TASK_FACE_HINTNODE" => Some(match &self.hint {
                Some(hint) if motor.face(Goal::Yaw(hint.yaw)) => TaskStatus::Complete,
                Some(_) => TaskStatus::Running,
                None => TaskStatus::Failed,
            }),
            "TASK_ACTBUSY_PLAY_BUSY_ANIM" => Some(
                if !self.enabled || self.end_busy_at > 0. && ctx.now >= self.end_busy_at {
                    TaskStatus::Complete
                } else {
                    TaskStatus::Running
                },
            ),
            _ => None,
        }
    }
}

/// CAI_HintManager::FindHint(npc, HINT_WORLD_WORK_POSITION, USE_GROUP | RANDOM or
/// NEAREST, range): the hints of the NPC's hint group (exact match) that are enabled,
/// unlocked, within range and allow the NPC's state (1 idle .. 3 combat).
pub struct HintCandidate {
    pub entity: usize,
    pub origin: Vec3,
    pub yaw: f32,
    pub hint_type: i32,
    pub group: String,
    pub activity: String,
    pub disabled: bool,
    pub min_state: i32,
    pub max_state: i32,
}
#[allow(clippy::too_many_arguments)]
pub fn find_hint(
    hints: &[HintCandidate],
    anims: &[BusyAnim],
    group: &str,
    origin: Vec3,
    range: f32,
    state: i32,
    locked: impl Fn(usize) -> bool,
    nearest: bool,
    random: f32,
) -> Option<BusyHint> {
    let valid: Vec<(&HintCandidate, &BusyAnim)> = hints
        .iter()
        .filter(|h| {
            h.hint_type == HINT_WORLD_WORK_POSITION
                && !h.disabled
                && h.group.eq_ignore_ascii_case(group)
                && h.origin.distance(origin) <= range
                && (h.min_state == 0 || state >= h.min_state)
                && (h.max_state == 0 || state <= h.max_state)
                && !locked(h.entity)
        })
        .filter_map(|h| Some((h, find_busy_anim(anims, &h.activity)?)))
        .collect();
    let pick = if nearest {
        valid.iter().min_by(|a, b| {
            a.0.origin
                .distance_squared(origin)
                .total_cmp(&b.0.origin.distance_squared(origin))
        })
    } else if valid.is_empty() {
        None
    } else {
        valid.get(((random * valid.len() as f32) as usize).min(valid.len() - 1))
    }?;
    Some(BusyHint {
        entity: pick.0.entity,
        origin: pick.0.origin,
        yaw: pick.0.yaw,
        anim: pick.1.clone(),
    })
}

#[cfg(test)]
mod tests {
    use super::super::schedule::tests::FakeMotor;
    use super::*;

    fn stand() -> Vec<BusyAnim> {
        parse_busy_anims(
            "\"ActBusy.txt\" { \"ACT_BUSY_STAND\" { \"busy_anim\" \"ACT_BUSY_STAND\" \
             \"min_time\" \"10.0\" \"max_time\" \"20.0\" \"interrupts\" \"BA_INT_DANGER\" } \
             \"seq_only\" { \"busy_sequence\" \"x\" } }",
        )
    }
    fn hints() -> Vec<HintCandidate> {
        [(-3876., -1008.), (-4000., -1536.), (-4064., -1968.)]
            .into_iter()
            .enumerate()
            .map(|(i, (x, y))| HintCandidate {
                entity: 824 + i,
                origin: Vec3::new(x, y, 72.),
                yaw: 284.,
                hint_type: 12,
                group: "terminal_cop_nodes".into(),
                activity: "ACT_BUSY_STAND".into(),
                disabled: false,
                min_state: 1,
                max_state: 3,
            })
            .collect()
    }

    #[test]
    fn busy_anims_and_hint_search() {
        let anims = stand();
        assert_eq!(anims.len(), 1);
        assert_eq!(anims[0].max_time, 20.);
        let origin = Vec3::new(-3740., -932., 72.);
        let h = hints();
        // Nearest picks the closest; random spreads over the three; locks and groups apply.
        let near = find_hint(
            &h,
            &anims,
            "terminal_cop_nodes",
            origin,
            9999.,
            2,
            |_| false,
            true,
            0.9,
        );
        assert_eq!(near.unwrap().entity, 824);
        let far = find_hint(
            &h,
            &anims,
            "terminal_cop_nodes",
            origin,
            9999.,
            2,
            |_| false,
            false,
            0.9,
        );
        assert_eq!(far.unwrap().entity, 826);
        let unlocked = find_hint(
            &h,
            &anims,
            "terminal_cop_nodes",
            origin,
            9999.,
            2,
            |e| e != 825,
            false,
            0.,
        );
        assert_eq!(unlocked.unwrap().entity, 825);
        assert!(find_hint(&h, &anims, "", origin, 9999., 2, |_| false, false, 0.).is_none());
        assert!(find_hint(
            &h,
            &anims,
            "terminal_cop_nodes",
            origin,
            100.,
            2,
            |_| false,
            false,
            0.
        )
        .is_none());
    }

    #[test]
    fn forced_actbusy_walks_faces_and_busies() {
        let mut s = Schedules::default();
        add_schedules(&mut s);
        let anims = stand();
        let mut npc = AiNpc::new(0);
        let mut a = ActBusy::default();
        let ctx = |now: f64| Context {
            now,
            random: 0.5,
            origin: Vec3::new(-3876., -1008., 72.),
            ..Default::default()
        };
        a.force(Some(823), 9999., None, false, &mut npc, 0.);
        assert!(a.can_select(&npc, 0.));
        a.candidate = find_hint(
            &hints(),
            &anims,
            "terminal_cop_nodes",
            Vec3::ZERO,
            1e5,
            1,
            |_| false,
            true,
            0.,
        );
        assert_eq!(
            a.select(&mut npc, &ctx(0.), "SCHED_IDLE_STAND"),
            Some("SCHED_ACTBUSY_START_BUSYING")
        );
        let mut m = FakeMotor::default();
        npc.schedule = Some("SCHED_ACTBUSY_BUSY".into());
        assert_eq!(
            a.start_task(
                &mut npc,
                "TASK_ACTBUSY_PLAY_BUSY_ANIM",
                &TaskArg::Number(0.),
                &ctx(1.),
                &mut m
            ),
            Some(TaskStatus::Running)
        );
        assert!(a.busy && !a.forced);
        // ACT_BUSY_STAND: 10-20 s (random 0.5: 15 s).
        assert!((a.end_busy_at - 16.).abs() < 1e-6);
        assert_eq!(a.outputs, vec!["OnNPCStartedBusy"]);
        assert_eq!(
            a.select(&mut npc, &ctx(5.), "SCHED_IDLE_STAND"),
            Some("SCHED_ACTBUSY_BUSY")
        );
        assert_eq!(
            a.select(&mut npc, &ctx(16.5), "SCHED_IDLE_STAND"),
            Some("SCHED_ACTBUSY_STOP_BUSYING")
        );
        a.start_task(
            &mut npc,
            "TASK_ACTBUSY_PLAY_EXIT",
            &TaskArg::Number(0.),
            &ctx(16.6),
            &mut m,
        );
        assert!(!a.busy && a.hint.is_none());
        assert_eq!(a.released, vec![824]);
        // The next search waits for the search time set at the forced selection (now + 2).
        assert!(a.wants_hint(16.7));
        // A base schedule other than idle (e.g. combat) leaves actbusy alone.
        assert_eq!(a.select(&mut npc, &ctx(17.), "SCHED_CHASE_ENEMY"), None);
    }
}
