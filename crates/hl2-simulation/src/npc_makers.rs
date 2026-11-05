//! npc_template_maker (SDK monstermaker.cpp CTemplateNPCMaker), template NPCs and
//! CAI_BaseNPC::StartNPC's floor drop (DESIGN 14s).
//!
//! MapEntity_ParseAllEntities removes NPCs with SF_NPC_TEMPLATE at map spawn (their
//! keyvalues become the maker's template); the Scene holds them killed. A maker's first
//! spawn activates that held entity; later spawns are new entities copied from it
//! (`copy_requests`, created by the host with DESIGN 14r runtime entities, then
//! `adopt`ed). StartNPC runs once per NPC on its first live tick.
use std::collections::{BTreeMap, BTreeSet};

use glam::Vec3;
use modkit_core::{Entity, World};

use crate::entities::Scene;
use crate::npc_probe::{ActorHull, Hull, NpcCollisionWorld, Query, NPC_SOLID_MASK};
use crate::physics::{npc_hull, Physics};

/// SF_NPC_FALL_TO_GROUND (ai_basenpc.h).
pub const SF_NPC_FALL_TO_GROUND: u32 = 1 << 2;
/// Classes whose Spawn skips NPCInit, so nothing puts them on the ground: a barnacle hangs from
/// its ceiling, whatever its spawnflags say.
fn stays_where_placed(class: &str) -> bool {
    class == "npc_barnacle"
}
/// SF_NPC_TEMPLATE: removed at map spawn, used by an npc_template_maker.
pub const SF_NPC_TEMPLATE: u32 = 1 << 11;
const SF_NPCMAKER_INF_CHILD: u32 = 32;
const SF_NPCMAKER_NO_DROP: u32 = 64;
const SF_NPCMAKER_HIDEFROMPLAYER: u32 = 128;
const SF_NPCMAKER_ALWAYSUSERADIUS: u32 = 256;
/// ai_moveprobe.cpp MOVE_HEIGHT_EPSILON.
const MOVE_HEIGHT_EPSILON: f32 = 0.0625;
/// sv_gravity.
const GRAVITY: f32 = 600.;
/// CBasePlayer::m_flFieldOfView (FInViewCone, 2D).
const PLAYER_FOV_DOT: f32 = 0.766;
const HUMAN: Hull = Hull {
    mins: Vec3::new(-13., -13., 0.),
    maxs: Vec3::new(13., 13., 72.),
};

fn number(e: &Entity, key: &str, default: f32) -> f32 {
    e.get(key)
        .and_then(|v| v.trim().parse::<f32>().ok())
        .unwrap_or(default)
}
fn flags(e: &Entity) -> u32 {
    number(e, "spawnflags", 0.) as i64 as u32
}
fn yaw(e: &Entity) -> f32 {
    e.get("angles")
        .and_then(modkit_core::parse_vec3)
        .map_or(0., |a| a.y)
}

/// npc_* entities that are CAI_BaseNPCs (IsTemplate is a CAI_BaseNPC override); the
/// makers and helicopter avoidance helpers share the prefix but are not NPCs.
pub fn is_npc_class(class: &str) -> bool {
    class.starts_with("npc_") && !class.ends_with("_maker") && !class.starts_with("npc_heli_")
}

/// Map NPCs carrying SF_NPC_TEMPLATE: not spawned, held for a maker.
pub fn template_npcs(world: &World) -> Vec<usize> {
    world
        .entities
        .iter()
        .enumerate()
        .filter(|(_, e)| is_npc_class(e.class()) && flags(e) & SF_NPC_TEMPLATE != 0)
        .map(|(id, _)| id)
        .collect()
}

#[derive(Clone, Debug)]
pub struct Maker {
    pub id: usize,
    /// The held template entity (TemplateName); None if missing.
    pub template: Option<usize>,
    /// Whether the held template has already been used for a spawn.
    pub template_used: bool,
    pub disabled: bool,
    pub next_think: Option<f64>,
    pub max_npcs: i32,
    pub max_live: i32,
    pub frequency: f32,
    pub flags: u32,
    pub radius: f32,
    pub destination_group: String,
    pub criterion_visibility: i32,
    pub criterion_distance: i32,
    pub min_spawn_distance: f32,
    pub live: Vec<usize>,
}
impl Maker {
    /// IsDepleted.
    fn depleted(&self) -> bool {
        !(self.flags & SF_NPCMAKER_INF_CHILD != 0 || self.max_npcs > 0)
    }
}

/// The viewer for spawn criteria (the local player).
#[derive(Clone, Copy, Debug)]
pub struct Viewer {
    pub eye: Vec3,
    pub forward: Vec3,
    pub feet: Vec3,
    pub crouched: bool,
    pub notarget: bool,
}

/// A maker spawn that needs a new entity copied from the held template (the template
/// was used already or another maker shares it): CTemplateNPCMaker::MakeNPC's
/// MapEntity_ParseEntity of the template data. The host creates it (runtime
/// entities) and calls `NpcMakers::adopt` with the new id.
#[derive(Clone, Debug, serde::Serialize)]
pub struct CopyRequest {
    pub maker: usize,
    pub template: usize,
    pub origin: Vec3,
    pub yaw: f32,
    /// The copy's spawnflags: SF_NPC_TEMPLATE removed, SF_NPC_FALL_TO_GROUND per the
    /// maker (NO_DROP keeps the template's, radius spawns add it).
    pub spawnflags: u32,
    pub destination: Option<usize>,
}

#[derive(Clone, Debug, Default)]
pub struct NpcMakers {
    pub makers: BTreeMap<usize, Maker>,
    /// Template targetname (lowercase) -> held entity.
    templates: BTreeMap<String, usize>,
    held: BTreeSet<usize>,
    /// NPCs whose StartNPC ran.
    started: BTreeSet<usize>,
    /// MOVETYPE_STEP NPCs off the ground: vertical speed.
    falling: BTreeMap<usize, f32>,
    /// info_npc_spawn_destination m_TimeNextAvailable.
    destination_free_at: BTreeMap<usize, f64>,
    rng: u32,
    pub diagnostics: MakerDiagnostics,
    /// Copies waiting for the host this tick.
    pub copy_requests: Vec<CopyRequest>,
}
#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct MakerDiagnostics {
    pub spawned: usize,
    pub floor_drops: usize,
    pub floor_failures: usize,
    pub landed: usize,
    /// Spawns made as new entities (copies of a held template).
    pub copies: usize,
    /// Copies the host could not create (entity limit, missing template model).
    pub copy_failures: usize,
}

impl NpcMakers {
    /// Hold every template NPC (killed, invisible) and read the makers' keyvalues.
    pub fn new(world: &World, states: &mut [crate::entities::State]) -> Self {
        let mut makers = Self {
            rng: 0x2545_f491,
            ..Default::default()
        };
        for id in template_npcs(world) {
            let state = &mut states[id];
            state.killed = true;
            state.visible = false;
            state.enabled = false;
            makers.held.insert(id);
            if let Some(name) = world.entities[id].get("targetname") {
                makers.templates.entry(name.to_lowercase()).or_insert(id);
            }
        }
        for (id, e) in world.entities.iter().enumerate() {
            if e.class() != "npc_template_maker" {
                continue;
            }
            let disabled = number(e, "StartDisabled", 0.) != 0.;
            let template = e
                .get("TemplateName")
                .and_then(|n| makers.templates.get(&n.to_lowercase()).copied());
            makers.makers.insert(
                id,
                Maker {
                    id,
                    template,
                    template_used: false,
                    disabled,
                    // CBaseNPCMaker::Spawn: MakerThink 0.1 s after spawn when enabled.
                    next_think: (!disabled).then_some(0.1),
                    max_npcs: number(e, "MaxNPCCount", 0.) as i32,
                    max_live: number(e, "MaxLiveChildren", 0.) as i32,
                    frequency: number(e, "SpawnFrequency", 0.),
                    // Infinite makers force corpse fading (SF_NPCMAKER_FADE).
                    flags: flags(e)
                        | if flags(e) & SF_NPCMAKER_INF_CHILD != 0 {
                            16
                        } else {
                            0
                        },
                    radius: number(e, "Radius", number(e, "radius", 0.)),
                    destination_group: e.get("DestinationGroup").unwrap_or("").to_owned(),
                    criterion_visibility: number(e, "CriterionVisibility", 2.) as i32,
                    criterion_distance: number(e, "CriterionDistance", 2.) as i32,
                    min_spawn_distance: number(e, "MinSpawnDistance", 0.),
                    live: Vec::new(),
                },
            );
        }
        makers
    }
    /// Whether an entity is a held template that has not been spawned.
    pub fn held(&self, id: usize) -> bool {
        self.held.contains(&id)
    }
    fn random(&mut self, n: usize) -> usize {
        // xorshift32; native uses the CRT rand().
        self.rng ^= self.rng << 13;
        self.rng ^= self.rng >> 17;
        self.rng ^= self.rng << 5;
        (self.rng as usize) % n.max(1)
    }

    /// One 15 ms tick: maker inputs, MakerThink, DeathNotice, StartNPC and step falling.
    pub fn tick(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        viewer: Option<Viewer>,
        dt: f32,
    ) {
        for (id, input, parameter) in std::mem::take(&mut scene.maker_inputs) {
            self.input(world, scene, physics, viewer, id, &input, &parameter);
        }
        let due: Vec<usize> = self
            .makers
            .values()
            .filter(|m| !m.disabled && m.next_think.is_some_and(|t| t <= scene.time))
            .map(|m| m.id)
            .collect();
        for id in due {
            // MakerThink: SetNextThink(curtime + frequency), then MakeNPC.
            let maker = self.makers.get_mut(&id).expect("due maker");
            maker.next_think = Some(scene.time + f64::from(maker.frequency.max(0.)));
            self.make_npc(world, scene, physics, viewer, id, false);
        }
        self.death_notices(scene);
        self.start_npcs(world, scene, physics);
        self.fall(world, scene, physics, dt);
    }

    #[allow(clippy::too_many_arguments)]
    fn input(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        viewer: Option<Viewer>,
        id: usize,
        input: &str,
        parameter: &str,
    ) {
        let Some(maker) = self.makers.get_mut(&id) else {
            return;
        };
        let int = parameter.trim().parse::<f32>().unwrap_or(0.) as i32;
        match input {
            "spawn" => {
                if !maker.depleted() {
                    self.make_npc(world, scene, physics, viewer, id, false);
                }
            }
            "enable" => {
                // Can't be enabled once depleted; thinks right away.
                if !maker.depleted() {
                    maker.disabled = false;
                    maker.next_think = Some(scene.time);
                }
            }
            "disable" => {
                maker.disabled = true;
                maker.next_think = None;
            }
            "toggle" => {
                if maker.disabled {
                    if !maker.depleted() {
                        maker.disabled = false;
                        maker.next_think = Some(scene.time);
                    }
                } else {
                    maker.disabled = true;
                    maker.next_think = None;
                }
            }
            "setmaxchildren" => maker.max_npcs = int,
            "addmaxchildren" => maker.max_npcs += int,
            "setmaxlivechildren" => maker.max_live = int,
            "setspawnfrequency" => maker.frequency = parameter.trim().parse().unwrap_or(0.),
            "changedestinationgroup" => maker.destination_group = parameter.trim().to_owned(),
            "setminimumspawndistance" => maker.min_spawn_distance = int as f32,
            "spawnnpcinradius" => {
                if !maker.depleted() {
                    self.make_npc(world, scene, physics, viewer, id, true);
                }
            }
            "spawnmultiple" => {
                let in_radius = maker.destination_group.is_empty() && maker.radius > 0.1;
                for _ in 0..int.max(0) {
                    self.make_npc(world, scene, physics, viewer, id, in_radius);
                }
            }
            other => scene.unsupported_input("npc_template_maker", other),
        }
    }

    /// Live NPC and player hulls (UTIL_TraceHull hits SOLID_BBOX NPCs and the player).
    fn transients(world: &World, scene: &Scene, viewer: Option<Viewer>) -> Vec<ActorHull> {
        let mut out: Vec<ActorHull> = world
            .entities
            .iter()
            .enumerate()
            .filter(|(id, e)| {
                let s = &scene.states[*id];
                is_npc_class(e.class()) && !s.killed && s.visible
            })
            .filter_map(|(id, e)| {
                npc_hull(e.class()).map(|(mins, maxs)| ActorHull {
                    entity: Some(id),
                    feet: scene.states[id].origin,
                    hull: Hull { mins, maxs },
                })
            })
            .collect();
        if let Some(v) = viewer {
            out.push(ActorHull {
                entity: None,
                feet: v.feet,
                hull: Hull {
                    mins: Vec3::new(-16., -16., 0.),
                    maxs: Vec3::new(16., 16., if v.crouched { 36. } else { 72. }),
                },
            });
        }
        out
    }

    /// HumanHullFits: a HULL_HUMAN trace from the spot 1 unit up is clear.
    fn human_hull_fits(physics: &Physics, transients: &[ActorHull], at: Vec3) -> bool {
        let trace = physics.npc_trace_hull(
            at,
            at + Vec3::Z,
            HUMAN,
            Query {
                contents_mask: NPC_SOLID_MASK,
                excluded_entities: &[],
                transients,
            },
        );
        trace.trace.fraction >= 1. && !trace.trace.start_solid
    }

    /// CanMakeNPC.
    fn can_make(
        &self,
        world: &World,
        scene: &Scene,
        physics: &Physics,
        viewer: Option<Viewer>,
        id: usize,
        ignore_solid: bool,
    ) -> bool {
        let maker = &self.makers[&id];
        if maker.max_live > 0 && maker.live.len() as i32 >= maker.max_live {
            return false;
        }
        let origin = world.entities[id].origin();
        let transients = Self::transients(world, scene, viewer);
        if !ignore_solid {
            // UTIL_EntitiesInBox(origin -/+ (34,34,0), FL_CLIENT|FL_NPC) with a
            // zero-height box at the maker's z; a solid one requires a human hull to fit
            // on the ground below.
            let lo = origin - Vec3::new(34., 34., 0.);
            let hi = origin + Vec3::new(34., 34., 0.);
            let blocked = transients.iter().any(|a| {
                let min = a.feet + a.hull.mins;
                let max = a.feet + a.hull.maxs;
                min.x <= hi.x
                    && max.x >= lo.x
                    && min.y <= hi.y
                    && max.y >= lo.y
                    && min.z <= hi.z
                    && max.z >= lo.z
            });
            if blocked {
                let start = origin + Vec3::Z * 2.;
                let end = origin - Vec3::Z * 8192.;
                let down = physics.npc_trace_hull(
                    start,
                    end,
                    HUMAN,
                    Query {
                        contents_mask: NPC_SOLID_MASK,
                        excluded_entities: &[],
                        transients: &transients,
                    },
                );
                let endpos = start + (end - start) * down.trace.fraction;
                if !Self::human_hull_fits(physics, &transients, endpos + Vec3::Z) {
                    return false;
                }
            }
        }
        if maker.flags & SF_NPCMAKER_HIDEFROMPLAYER != 0 {
            if let Some(v) = viewer {
                if in_view_cone(v, origin)
                    && player_visible(world, physics, v, origin)
                    && !v.notarget
                {
                    return false;
                }
            }
        }
        true
    }

    /// FindSpawnDestination.
    fn find_destination(
        &mut self,
        world: &World,
        scene: &Scene,
        physics: &Physics,
        viewer: Option<Viewer>,
        id: usize,
    ) -> Option<usize> {
        let v = viewer?;
        let maker = self.makers[&id].clone();
        let group = maker.destination_group.to_lowercase();
        let transients = Self::transients(world, scene, viewer);
        let mut candidates = Vec::new();
        for (dest, e) in world.entities.iter().enumerate() {
            if !e
                .get("targetname")
                .is_some_and(|n| n.eq_ignore_ascii_case(&group))
                || e.class() != "info_npc_spawn_destination"
                || scene.states[dest].killed
                || self
                    .destination_free_at
                    .get(&dest)
                    .is_some_and(|t| *t > scene.time)
            {
                continue;
            }
            let at = e.origin();
            if maker.criterion_visibility != 2 {
                let visible = player_visible(world, physics, v, at)
                    || player_visible(world, physics, v, at + Vec3::Z * HUMAN.maxs.z);
                let valid = if maker.criterion_visibility == 0 {
                    visible
                } else {
                    !visible || v.notarget
                };
                if !valid {
                    continue;
                }
            }
            candidates.push((dest, at));
        }
        if candidates.is_empty() {
            return None;
        }
        if maker.criterion_distance == 2 {
            // Five random tries for a spot where a human hull fits.
            for _ in 0..5 {
                let (dest, at) = candidates[self.random(candidates.len())];
                if Self::human_hull_fits(physics, &transients, at) {
                    return Some(dest);
                }
            }
            return None;
        }
        let nearest = maker.criterion_distance == 0;
        let mut best: Option<(usize, f32)> = None;
        for (dest, at) in candidates {
            let distance = (at - v.feet).length();
            if maker.min_spawn_distance != 0. && maker.min_spawn_distance > distance {
                continue;
            }
            let better = match best {
                None => {
                    if nearest {
                        distance < f32::MAX
                    } else {
                        distance > 0.
                    }
                }
                Some((_, d)) => {
                    if nearest {
                        distance < d
                    } else {
                        distance > d
                    }
                }
            };
            if better && Self::human_hull_fits(physics, &transients, at) {
                best = Some((dest, distance));
            }
        }
        best.map(|(dest, _)| dest)
    }

    /// FindSpotForNPCInRadius: an 18-degree fan on the radius, ground below, hull clear
    /// 10 units up (CheckStandPosition's corner support check is not modeled).
    #[allow(clippy::too_many_arguments)]
    fn spot_in_radius(
        world: &World,
        scene: &Scene,
        physics: &Physics,
        viewer: Option<Viewer>,
        center: Vec3,
        radius: f32,
        hull: Hull,
        npc: usize,
    ) -> Option<Vec3> {
        let transients = Self::transients(world, scene, viewer);
        let mut yaw = 0f32;
        while yaw < 360. {
            let dir = Vec3::new(yaw.to_radians().cos(), yaw.to_radians().sin(), 0.);
            yaw += 18.;
            let test = center + dir * radius;
            let Some(hit) = physics.shot_ray(test, -Vec3::Z, 8192., &[npc]) else {
                continue;
            };
            let trace = physics.npc_trace_hull(
                hit.position,
                hit.position + Vec3::Z * 10.,
                hull,
                Query {
                    contents_mask: NPC_SOLID_MASK,
                    excluded_entities: &[npc],
                    transients: &transients,
                },
            );
            if trace.trace.fraction >= 1. && !trace.trace.start_solid {
                return Some(hit.position + Vec3::Z * 10.);
            }
        }
        None
    }

    /// MakeNPC / MakeNPCInRadius.
    fn make_npc(
        &mut self,
        world: &World,
        scene: &mut Scene,
        physics: &Physics,
        viewer: Option<Viewer>,
        id: usize,
        force_radius: bool,
    ) {
        let maker = self.makers[&id].clone();
        let radius_mode =
            force_radius || (maker.radius != 0. && maker.flags & SF_NPCMAKER_ALWAYSUSERADIUS != 0);
        let has_group = !maker.destination_group.is_empty();
        if !self.can_make(world, scene, physics, viewer, id, radius_mode || has_group) {
            return;
        }
        let destination = if !radius_mode && has_group {
            match self.find_destination(world, scene, physics, viewer, id) {
                Some(d) => Some(d),
                None => return,
            }
        } else {
            None
        };
        let Some(template) = maker.template else {
            scene.unsupported_input("npc_template_maker", "missing-template");
            return;
        };
        // The held template exists once; makers sharing a TemplateName and every later
        // spawn create a new entity from it.
        let copy = maker.template_used || !self.held.contains(&template);
        let class = world.entities[template].class();
        let maker_entity = &world.entities[id];
        let (origin, yaw) = if radius_mode {
            let (mins, maxs) = npc_hull(class).unwrap_or((HUMAN.mins, HUMAN.maxs));
            match Self::spot_in_radius(
                world,
                scene,
                physics,
                viewer,
                maker_entity.origin(),
                maker.radius,
                Hull { mins, maxs },
                template,
            ) {
                Some(spot) => (spot, yaw(&world.entities[template])),
                // Failed to place: the copy is removed.
                None => return,
            }
        } else if let Some(dest) = destination {
            let e = &world.entities[dest];
            (e.origin(), yaw(e))
        } else {
            (maker_entity.origin(), yaw(maker_entity))
        };
        // Radius spawns add SF_NPC_FALL_TO_GROUND; other spawns remove it unless the
        // maker has NO_DROP (the template's own flag then stays).
        let template_flags = flags(&world.entities[template]);
        let falls = if radius_mode {
            true
        } else if maker.flags & SF_NPCMAKER_NO_DROP == 0 {
            false
        } else {
            template_flags & SF_NPC_FALL_TO_GROUND != 0
        };
        if maker.flags & SF_NPCMAKER_INF_CHILD == 0 {
            // Counted now so this tick's later checks see a queued copy too.
            self.makers.get_mut(&id).expect("maker").max_npcs -= 1;
        }
        if copy {
            let spawnflags = (template_flags & !SF_NPC_TEMPLATE & !SF_NPC_FALL_TO_GROUND)
                | if falls { SF_NPC_FALL_TO_GROUND } else { 0 };
            self.copy_requests.push(CopyRequest {
                maker: id,
                template,
                origin,
                yaw,
                spawnflags,
                destination,
            });
            return;
        }
        let state = &mut scene.states[template];
        state.killed = false;
        state.visible = true;
        state.enabled = true;
        state.origin = origin;
        state.rotation = crate::physics::angles(Vec3::new(0., yaw, 0.));
        self.held.remove(&template);
        self.makers.get_mut(&id).expect("maker").template_used = true;
        self.finish_spawn(world, scene, id, template, falls, destination);
    }

    /// The host created a requested copy as entity `child` (None: it could not).
    pub fn adopt(
        &mut self,
        world: &World,
        scene: &mut Scene,
        request: &CopyRequest,
        child: Option<usize>,
    ) {
        let Some(child) = child else {
            self.diagnostics.copy_failures += 1;
            // Not made: the count goes back.
            if let Some(m) = self.makers.get_mut(&request.maker) {
                if m.flags & SF_NPCMAKER_INF_CHILD == 0 {
                    m.max_npcs += 1;
                }
            }
            return;
        };
        self.diagnostics.copies += 1;
        let falls = request.spawnflags & SF_NPC_FALL_TO_GROUND != 0;
        self.finish_spawn(
            world,
            scene,
            request.maker,
            child,
            falls,
            request.destination,
        );
    }

    /// The rest of MakeNPC for a spawned child: destination reuse, StartNPC/fall state,
    /// live children, OnSpawnNPC (activator = the child) and OnAllSpawned.
    fn finish_spawn(
        &mut self,
        world: &World,
        scene: &mut Scene,
        id: usize,
        child: usize,
        falls: bool,
        destination: Option<usize>,
    ) {
        if let Some(dest) = destination {
            // CNPCSpawnDestination::OnSpawnedNPC.
            if world.entities[dest]
                .get("RenameNPC")
                .is_some_and(|n| !n.is_empty())
            {
                scene.unsupported_input("info_npc_spawn_destination", "RenameNPC");
            }
            scene.fire(dest, "OnSpawnNPC", child);
            let delay = number(&world.entities[dest], "ReuseDelay", 1.);
            self.destination_free_at
                .insert(dest, scene.time + f64::from(delay));
        }
        self.started.remove(&child);
        if falls {
            self.started.insert(child);
            self.falling.insert(child, 0.);
        }
        self.diagnostics.spawned += 1;
        let maker = self.makers.get_mut(&id).expect("maker");
        maker.live.push(child);
        scene.fire(id, "OnSpawnNPC", child);
        if maker.flags & SF_NPCMAKER_INF_CHILD == 0 && maker.depleted() {
            scene.fire(id, "OnAllSpawned", id);
            maker.disabled = true;
            maker.next_think = None;
        }
    }

    /// CBaseCombatCharacter::UpdateOnRemove -> DeathNotice for removed/dead children.
    fn death_notices(&mut self, scene: &mut Scene) {
        let mut fired = Vec::new();
        for maker in self.makers.values_mut() {
            let before = maker.live.len();
            maker.live.retain(|child| !scene.states[*child].killed);
            for _ in maker.live.len()..before {
                if maker.live.is_empty() {
                    fired.push((maker.id, "OnAllLiveChildrenDead"));
                    if maker.flags & SF_NPCMAKER_INF_CHILD == 0 && maker.depleted() {
                        fired.push((maker.id, "OnAllSpawnedDead"));
                    }
                }
            }
        }
        for (id, output) in fired {
            scene.fire(id, output, id);
        }
    }

    /// StartNPC on each NPC's first live tick.
    fn start_npcs(&mut self, world: &World, scene: &mut Scene, physics: &Physics) {
        for (id, e) in world.entities.iter().enumerate() {
            let s = &scene.states[id];
            if !is_npc_class(e.class()) || s.killed || !s.visible || self.started.contains(&id) {
                continue;
            }
            self.started.insert(id);
            if stays_where_placed(e.class()) {
                continue;
            }
            let Some((mins, maxs)) = npc_hull(e.class()) else {
                continue;
            };
            let parented = e.get("parentname").is_some_and(|p| !p.is_empty());
            let rappel = e
                .get("waitingtorappel")
                .is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("yes"));
            if parented {
                continue;
            }
            if flags(e) & SF_NPC_FALL_TO_GROUND != 0 || rappel {
                // SetGroundEntity(NULL): MOVETYPE_STEP gravity drops it.
                if !rappel {
                    self.falling.insert(id, 0.);
                }
                continue;
            }
            let origin = scene.states[id].origin;
            match floor_point(
                physics,
                origin + Vec3::Z * 0.1,
                Hull { mins, maxs },
                id,
                0.,
                -2048.,
            ) {
                Ok(floor) => {
                    scene.states[id].origin = floor;
                    self.diagnostics.floor_drops += 1;
                }
                Err(start) => {
                    // "NPC stuck in wall--level design error"; keeps the start point.
                    scene.states[id].origin = start;
                    self.diagnostics.floor_failures += 1;
                }
            }
        }
    }

    /// PhysicsStepRunTimestep for NPCs off the ground: half gravity before and after the
    /// move; lands on the first NPC-solid surface below.
    fn fall(&mut self, world: &World, scene: &mut Scene, physics: &Physics, dt: f32) {
        let mut landed = Vec::new();
        for (&id, speed) in self.falling.iter_mut() {
            let s = &scene.states[id];
            if s.killed || !s.visible || s.scripted_by.is_some() {
                continue;
            }
            let Some((mins, maxs)) = npc_hull(world.entities[id].class()) else {
                landed.push(id);
                continue;
            };
            *speed -= GRAVITY * dt * 0.5;
            let start = s.origin;
            let end = start + Vec3::Z * (*speed * dt).min(-0.01);
            let trace = physics.npc_trace_hull(
                start,
                end,
                Hull { mins, maxs },
                Query {
                    contents_mask: NPC_SOLID_MASK,
                    excluded_entities: &[id],
                    transients: &[],
                },
            );
            if trace.trace.start_solid {
                landed.push(id);
                continue;
            }
            let at = start + (end - start) * trace.trace.fraction;
            scene.states[id].origin = at;
            if trace.trace.fraction < 1. {
                landed.push(id);
            } else {
                *speed -= GRAVITY * dt * 0.5;
            }
        }
        for id in landed {
            self.falling.remove(&id);
            self.diagnostics.landed += 1;
        }
    }
    pub fn falling(&self, id: usize) -> bool {
        self.falling.contains_key(&id)
    }
}

/// Runs the Scene's makers for one tick (they live in the Scene so that every Scene
/// holds its templates from load).
pub fn tick(world: &World, scene: &mut Scene, physics: &Physics, viewer: Option<Viewer>, dt: f32) {
    let mut makers = std::mem::take(&mut scene.makers);
    makers.tick(world, scene, physics, viewer, dt);
    scene.makers = makers;
}

/// CAI_MoveProbe::FloorPoint: a zero-height box of the hull's footprint from
/// `start.z + start_z + MOVE_HEIGHT_EPSILON` down to `start.z + end_z`. Ok(floor) on a
/// hit; Err(start) when nothing was hit or the box is stuck.
pub fn floor_point(
    physics: &Physics,
    start: Vec3,
    hull: Hull,
    exclude: usize,
    start_z: f32,
    end_z: f32,
) -> Result<Vec3, Vec3> {
    let pizza = Hull {
        mins: hull.mins,
        maxs: Vec3::new(hull.maxs.x, hull.maxs.y, hull.mins.z),
    };
    let excluded = [exclude];
    let query = Query {
        contents_mask: NPC_SOLID_MASK,
        excluded_entities: &excluded,
        transients: &[],
    };
    let down = Vec3::new(start.x, start.y, start.z + end_z);
    let mut up = Vec3::new(start.x, start.y, start.z + start_z + MOVE_HEIGHT_EPSILON);
    let mut trace = physics.npc_trace_hull(up, down, pizza, query);
    if trace.trace.start_solid {
        // (The started-in-object exemption needs |start - origin| < 0.1; StartNPC passes
        // origin + 0.1, so it never applies here.)
        up.z = start.z + MOVE_HEIGHT_EPSILON;
        trace = physics.npc_trace_hull(up, down, pizza, query);
    }
    if trace.trace.fraction >= 1. || trace.trace.all_solid {
        return Err(start);
    }
    Ok(up + (down - up) * trace.trace.fraction)
}

/// CBaseCombatCharacter::FInViewCone (2D) with the player's 0.766.
fn in_view_cone(v: Viewer, spot: Vec3) -> bool {
    let los = (spot - v.eye) * Vec3::new(1., 1., 0.);
    let facing = (v.forward * Vec3::new(1., 1., 0.)).normalize_or_zero();
    los.normalize_or_zero().dot(facing) > PLAYER_FOV_DOT
}
/// CBaseEntity::FVisible(point): MASK_BLOCKLOS from the eye (NPC bodies do not block).
fn player_visible(world: &World, physics: &Physics, v: Viewer, spot: Vec3) -> bool {
    use crate::ai::senses::SightWorld;
    crate::ai::host::PhysicsSight { physics, world }.visible(v.eye, spot, &[])
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_barnacle_is_never_put_on_the_ground() {
        assert!(stays_where_placed("npc_barnacle"));
        assert!(!stays_where_placed("npc_zombie"));
    }

    use super::*;
    use modkit_core::{Brush, Plane};

    fn cube(min: Vec3, max: Vec3) -> Brush {
        Brush {
            contents: 1,
            planes: [
                (Vec3::X, max.x),
                (-Vec3::X, -min.x),
                (Vec3::Y, max.y),
                (-Vec3::Y, -min.y),
                (Vec3::Z, max.z),
                (-Vec3::Z, -min.z),
            ]
            .into_iter()
            .map(|(normal, distance)| Plane { normal, distance })
            .collect(),
        }
    }
    fn entity(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        }
    }
    fn floor_world(entities: Vec<Entity>) -> World {
        World {
            // Floor top at z 0.
            brushes: vec![cube(
                Vec3::new(-2000., -2000., -64.),
                Vec3::new(2000., 2000., 0.),
            )],
            entities,
            ..Default::default()
        }
    }

    #[test]
    fn floor_point_drops_onto_the_floor_and_fails_without_one() {
        let world = floor_world(vec![]);
        let physics = Physics::new(&world);
        let hull = Hull {
            mins: Vec3::new(-13., -13., 0.),
            maxs: Vec3::new(13., 13., 72.),
        };
        let floor = floor_point(&physics, Vec3::new(0., 0., 40.1), hull, 99, 0., -2048.).unwrap();
        assert!(floor.z.abs() < 0.1, "{floor:?}");
        // Nothing within 2048 units below.
        let start = Vec3::new(0., 0., 3000.);
        assert_eq!(
            floor_point(&physics, start, hull, 99, 0., -2048.),
            Err(start)
        );
        // Embedded in the floor: the retry from the start also starts solid -> all solid.
        let inside = Vec3::new(0., 0., -10.);
        assert!(floor_point(&physics, inside, hull, 99, 0., -2048.).is_err());
    }

    #[test]
    fn template_npcs_are_held_and_the_maker_spawns_one_on_the_floor() {
        let mut world = floor_world(vec![
            entity(&[("classname", "worldspawn")]),
            entity(&[
                ("classname", "npc_metropolice"),
                ("targetname", "cop_template"),
                ("spawnflags", "2048"),
                ("origin", "0 0 300"),
            ]),
            entity(&[
                ("classname", "npc_template_maker"),
                ("targetname", "maker"),
                ("TemplateName", "cop_template"),
                ("origin", "100 50 40"),
                ("angles", "0 90 0"),
                ("StartDisabled", "1"),
                ("MaxNPCCount", "1"),
                ("MaxLiveChildren", "1"),
                ("SpawnFrequency", "5"),
            ]),
            // A second maker of the same template (d1_trainstation_02 #383/#1040).
            entity(&[
                ("classname", "npc_template_maker"),
                ("targetname", "maker2"),
                ("TemplateName", "cop_template"),
                ("origin", "-100 0 40"),
                ("StartDisabled", "1"),
                ("MaxNPCCount", "1"),
            ]),
        ]);
        let mut scene = Scene::new(&world);
        let physics = Physics::new(&world);
        assert!(scene.states[1].killed && scene.makers.held(1));
        // Disabled: nothing happens.
        tick(&world, &mut scene, &physics, None, 0.015);
        assert!(scene.states[1].killed);
        scene.send_named("maker", "Enable", "", 0.);
        scene.tick(&world, Vec3::splat(9999.), 0.015);
        tick(&world, &mut scene, &physics, None, 0.015);
        let makers = &scene.makers;
        let s = &scene.states[1];
        assert!(!s.killed && s.visible, "spawned");
        // MakeNPC (no NO_DROP): at the maker, then StartNPC slams it to the floor.
        assert!(
            (s.origin - Vec3::new(100., 50., 0.)).length() < 0.1,
            "{:?}",
            s.origin
        );
        assert_eq!(makers.diagnostics.spawned, 1);
        assert_eq!(makers.diagnostics.floor_drops, 1);
        // Depleted after one: OnAllSpawned and no further thinking.
        assert!(makers.makers[&2].disabled && makers.makers[&2].depleted());
        // The other maker spawns a new entity copied from the template (DESIGN 14r):
        // the maker queues it, the host creates it and the maker adopts it.
        scene.send_named("maker2", "Spawn", "", 0.);
        scene.tick(&world, Vec3::splat(9999.), 0.015);
        tick(&world, &mut scene, &physics, None, 0.015);
        assert!((scene.states[1].origin.x - 100.).abs() < 0.1);
        let requests = std::mem::take(&mut scene.makers.copy_requests);
        assert_eq!(requests.len(), 1);
        let request = &requests[0];
        assert_eq!((request.maker, request.template), (3, 1));
        assert_eq!(request.origin, Vec3::new(-100., 0., 40.));
        // SF_NPC_TEMPLATE removed; no NO_DROP: no fall flag.
        assert_eq!(request.spawnflags, 0);
        let child = world.entities.len();
        let (copy, _) =
            crate::ai::spawn::instantiate(&world, 1, child, request.origin, request.yaw);
        let id = scene.spawn_entity(&mut world, copy);
        let mut makers = std::mem::take(&mut scene.makers);
        makers.adopt(&world, &mut scene, request, Some(id));
        scene.makers = makers;
        assert_eq!(scene.makers.makers[&3].live, vec![id]);
        assert_eq!(scene.makers.diagnostics.copies, 1);
        assert!(scene.makers.makers[&3].disabled);
        // StartNPC slams the copy to the floor on its first tick.
        tick(&world, &mut scene, &physics, None, 0.015);
        assert!(
            scene.states[id].origin.z.abs() < 0.1,
            "{:?}",
            scene.states[id].origin
        );
        // The first child dies: DeathNotice -> live count 0.
        scene.states[1].killed = true;
        tick(&world, &mut scene, &physics, None, 0.015);
        assert!(scene.makers.makers[&2].live.is_empty());
        assert_eq!(scene.makers.makers[&3].live, vec![id]);
    }

    #[test]
    fn fall_to_ground_npcs_fall_under_step_gravity_and_land() {
        let world = floor_world(vec![
            entity(&[("classname", "worldspawn")]),
            entity(&[
                ("classname", "npc_citizen"),
                ("spawnflags", "4"),
                ("origin", "0 0 100"),
            ]),
        ]);
        let mut scene = Scene::new(&world);
        let physics = Physics::new(&world);
        tick(&world, &mut scene, &physics, None, 0.015);
        assert!(scene.makers.falling(1));
        // 100 units under 600 u/s^2: sqrt(2 * 100 / 600) = 0.58 s.
        let mut ticks = 0;
        while scene.makers.falling(1) && ticks < 200 {
            tick(&world, &mut scene, &physics, None, 0.015);
            ticks += 1;
        }
        assert!((35..=45).contains(&ticks), "{ticks} ticks");
        assert!(
            scene.states[1].origin.z.abs() < 0.2,
            "{:?}",
            scene.states[1].origin
        );
    }
}
