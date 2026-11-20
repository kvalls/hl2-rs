//! Triggers and touches: soundscapes, trigger_hurt, filters and player containment.

use super::{number, HurtState, Scene, PLAYER};
use glam::Vec3;
use modkit_core::World;

impl Scene {
    /// SDK FrameUpdatePostEntityThink soundscape pass for the player's ear (eye) position,
    /// after player movement; a newly active env_soundscape fires OnPlay.
    pub fn update_soundscape(&mut self, world: &World, ear: Vec3) {
        let states = &self.states;
        let enabled = |id: usize| states.get(id).is_some_and(|s| s.enabled && !s.killed);
        if let Some(id) = self.soundscape.update(world, ear, enabled) {
            self.fire(id, "OnPlay", id);
        }
    }

    /// SDK CTriggerHurt: Touch schedules HurtThink at once; HurtAllTouchers deals
    /// damage x 0.5 to every toucher that passes the filters every 0.5 s while it hurts
    /// anyone (damagemodel 1 doubles up to damagecap and forgives after 3 s without a
    /// hit); EndTouch deals damage x 0.5 to a toucher the last think did not hurt.
    pub(super) fn trigger_hurt(&mut self, world: &World, id: usize, touching: Vec<usize>) {
        let e = self.ent(world, id);
        let base = number(e, "damage", 0.);
        let cap = number(e, "damagecap", 20.);
        let doubling = number(e, "damagemodel", 0.) as u32 == 1;
        let time = self.time;
        let mut hurt = self.hurt.remove(&id).unwrap_or(HurtState {
            damage: base,
            next: None,
            reset_at: 0.,
            hurt: Vec::new(),
            touching: Vec::new(),
        });
        if !touching.is_empty() && hurt.next.is_none() {
            hurt.next = Some(time);
        }
        if hurt.next.is_some_and(|next| time >= next) {
            hurt.hurt.clear();
            let amount = hurt.damage * 0.5;
            let mut count = 0;
            for &target in &touching {
                if self.hurt_entity(world, id, target, amount) {
                    hurt.hurt.push(target);
                    count += 1;
                }
            }
            if doubling {
                if count == 0 {
                    if time > hurt.reset_at {
                        hurt.damage = base;
                    }
                } else {
                    hurt.damage = (hurt.damage * 2.).min(cap);
                    hurt.reset_at = time + 3.;
                }
            }
            hurt.next = (count > 0).then_some(time + 0.5);
        }
        for target in std::mem::take(&mut hurt.touching) {
            if !touching.contains(&target)
                && !hurt.hurt.contains(&target)
                && self.hurt_entity(world, id, target, hurt.damage * 0.5)
            {
                hurt.hurt.push(target);
            }
        }
        hurt.touching = touching;
        self.hurt.insert(id, hurt);
    }

    /// CTriggerHurt::HurtEntity: filters and m_takedamage, then TakeDamage (negative
    /// damage heals); the damage comes from the trigger's origin. OnHurtPlayer/OnHurt.
    fn hurt_entity(&mut self, world: &World, id: usize, target: usize, amount: f32) -> bool {
        let e = self.ent(world, id);
        let flags = number(e, "spawnflags", 0.) as u32;
        let kind = number(e, "damagetype", 0.) as u32;
        let player = target == PLAYER;
        let passes = flags & 0x40 != 0 || flags & if player { 1 } else { 2 } != 0;
        let alive = if player {
            !self.player_dead
        } else {
            self.states.get(target).is_some_and(|s| !s.killed)
        };
        if !passes || !alive {
            return false;
        }
        if player {
            self.player_damage.push(crate::player_damage::DamageInfo {
                amount,
                kind,
                inflictor: self.states[id].origin,
            });
            self.fire(id, "OnHurtPlayer", usize::MAX);
        } else {
            self.entity_damage.push(crate::projectiles::Damage {
                target: crate::projectiles::DamageTarget::Entity(target),
                amount,
                kind,
                dissolve: false,
                direction: Vec3::ZERO,
                origin: self.states[id].origin,
                attacker: crate::projectiles::Attacker::None,
            });
            self.fire(id, "OnHurt", target);
        }
        true
    }

    /// CBaseFilter::PassesFilter for a trigger's filter entity: filter_activator_name
    /// (targetname) and filter_activator_class (classname), with Negated; other filter
    /// classes (filter_multi, ...) pass, not modeled. `other` usize::MAX is the player.
    pub(super) fn passes_filter(&self, world: &World, filter: &str, other: usize) -> bool {
        let Some((_, f)) = world.entities.iter().enumerate().find(|(i, x)| {
            !self.states[*i].killed
                && x.class().starts_with("filter_")
                && x.get("targetname")
                    .is_some_and(|n| n.eq_ignore_ascii_case(filter))
        }) else {
            return true;
        };
        // FIELD_BOOLEAN: atoi, so Hammer's "Allow entities that match criteria" is 0.
        let negated = f
            .get("Negated")
            .and_then(|v| v.trim().parse::<i32>().ok())
            .unwrap_or(0)
            != 0;
        let (name, class) = if other == usize::MAX {
            ("", "player")
        } else {
            let o = self.ent(world, other);
            (o.get("targetname").unwrap_or(""), o.class())
        };
        let matched = match f.class() {
            "filter_activator_name" => f
                .get("filtername")
                .is_some_and(|n| !n.is_empty() && n.eq_ignore_ascii_case(name)),
            "filter_activator_class" => f
                .get("filterclass")
                .is_some_and(|c| c.eq_ignore_ascii_case(class)),
            _ => return true,
        };
        matched != negated
    }

    pub(super) fn player_inside(&self, world: &World, id: usize, feet: Vec3) -> bool {
        self.touches(world, id, feet + Vec3::Z * 36., Vec3::new(16., 16., 36.))
    }

    /// A box (center, half extents) overlapping the entity's brush model.
    pub(super) fn touches(&self, world: &World, id: usize, center: Vec3, half: Vec3) -> bool {
        let e = self.ent(world, id);
        let Some(model) = e
            .get("model")
            .and_then(|s| s.strip_prefix('*'))
            .and_then(|s| s.parse::<usize>().ok())
            .and_then(|id| world.brush_models.iter().find(|m| m.id == id))
        else {
            return false;
        };
        let state = &self.states[id];
        model.brushes.iter().any(|b| {
            !b.planes.is_empty()
                && b.planes.iter().all(|p| {
                    let normal = state.rotation * p.normal;
                    normal.dot(center - state.origin) <= p.distance + normal.abs().dot(half)
                })
        })
    }
}

/// Entity classes that move with VPhysics (MOVETYPE_VPHYSICS) for trigger touches.
fn is_vphysics_class(class: &str) -> bool {
    class.starts_with("prop_physics")
        || class.starts_with("weapon_")
        || class.starts_with("item_")
        || matches!(
            class,
            "physics_prop" | "func_physbox" | "func_physbox_multiplayer" | "prop_ragdoll"
        )
}

/// What a trigger may be touched by, gathered at most once per tick in entity order: classes
/// never change, while killed, visible and inside are tested per trigger.
#[derive(Default)]
pub(super) struct TouchCandidates {
    npcs: Option<Vec<usize>>,
    objects: Option<Vec<usize>>,
}

impl Scene {
    /// A trigger's touches this tick: the player, NPCs and VPhysics objects inside it, its
    /// touch and trigger outputs, trigger_hurt's damage and trigger_changelevel.
    pub(super) fn touch_trigger(
        &mut self,
        world: &World,
        id: usize,
        player_feet: Vec3,
        candidates: &mut TouchCandidates,
    ) {
        let e = self.ent(world, id);
        // CBaseTrigger PassesTriggerFilters: clients (1), NPCs (2), physics objects
        // (8, MOVETYPE_VPHYSICS), everything (0x40), then the trigger's filter
        // entity (filtername). Other trigger classes keep the earlier player-only
        // touch.
        let gated = matches!(e.class(), "trigger_once" | "trigger_multiple");
        let flags = number(e, "spawnflags", 1.) as u32;
        let clients = !gated || flags & (1 | 0x40) != 0;
        let npcs = gated && flags & (2 | 0x40) != 0;
        let physics_objects = gated && flags & (8 | 0x40) != 0;
        let filter = gated
            .then(|| e.get("filtername").map(str::trim).filter(|f| !f.is_empty()))
            .flatten()
            .map(str::to_owned);
        let passes = |scene: &Self, other: usize| {
            filter
                .as_deref()
                .is_none_or(|f| scene.passes_filter(world, f, other))
        };
        let player =
            clients && self.player_inside(world, id, player_feet) && passes(self, usize::MAX);
        let npc = if npcs && !player {
            let candidates = candidates.npcs.get_or_insert_with(|| {
                self.entities_where(world, |class| class.starts_with("npc_"))
            });
            candidates.iter().copied().find(|&n| {
                !self.states[n].killed
                    && self.states[n].visible
                    && self.player_inside(world, id, self.states[n].origin)
                    && passes(self, n)
            })
        } else {
            None
        };
        // A VPhysics object's origin inside the brush (small-object test).
        let object = if physics_objects && !player && npc.is_none() {
            let candidates = candidates
                .objects
                .get_or_insert_with(|| self.entities_where(world, is_vphysics_class));
            candidates.iter().copied().find(|&n| {
                !self.states[n].killed
                    && self.states[n].visible
                    && self.touches(world, id, self.states[n].origin, Vec3::splat(2.))
                    && passes(self, n)
            })
        } else {
            None
        };
        let inside = player || npc.is_some() || object.is_some();
        let activator = npc.or(object).unwrap_or(usize::MAX);
        if inside && !self.states[id].touching {
            self.diagnostics.trigger_entries += 1;
            self.fire(id, "OnStartTouch", activator);
            self.fire(id, "OnStartTouchAll", activator);
        }
        if !inside && self.states[id].touching {
            self.fire(id, "OnEndTouch", activator);
            self.fire(id, "OnEndTouchAll", activator);
        }
        if inside
            && gated
            && self.time - self.states[id].last_trigger >= number(e, "wait", 0.2) as f64
        {
            self.fire(id, "OnTrigger", activator);
            self.states[id].last_trigger = self.time;
            if e.class() == "trigger_once" {
                self.states[id].enabled = false;
            }
        }
        if e.class() == "trigger_hurt" {
            self.hurt_touching(world, id, player_feet, candidates);
        }
        // SF_CHANGELEVEL_NOTOUCH (0x2) leaves the ChangeLevel input available,
        // but must not change maps when the player overlaps the brush.
        if inside
            && e.class() == "trigger_changelevel"
            && number(e, "spawnflags", 0.) as u32 & 2 == 0
            && !self.states[id].touching
        {
            self.send(id, "ChangeLevel", "");
        }
        self.states[id].touching = inside;
    }
    /// trigger_hurt damages every touching player and NPC, before the trigger filters (Touch).
    fn hurt_touching(
        &mut self,
        world: &World,
        id: usize,
        player_feet: Vec3,
        candidates: &mut TouchCandidates,
    ) {
        let npcs = candidates
            .npcs
            .get_or_insert_with(|| self.entities_where(world, |class| class.starts_with("npc_")));
        let mut touching: Vec<usize> = (npcs.iter().copied())
            .filter(|&n| {
                !self.states[n].killed && self.player_inside(world, id, self.states[n].origin)
            })
            .collect();
        if self.player_inside(world, id, player_feet) {
            touching.insert(0, PLAYER);
        }
        self.trigger_hurt(world, id, touching);
    }
    /// The entities (scene states) whose class passes `test`, in entity order.
    fn entities_where(&self, world: &World, test: impl Fn(&str) -> bool) -> Vec<usize> {
        (0..self.states.len())
            .filter(|&n| test(self.ent(world, n).class()))
            .collect()
    }
}
