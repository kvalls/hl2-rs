//! Zombies coming apart on the host side: BecomeTorso's replacement entity and legs, and the
//! headcrab a zombie releases.
use super::Gameplay;
use glam::Vec3;
use hl2_simulation::physics::Physics;

/// CNPC_BaseZombie::HeadcrabFits: a tiny hull 1 unit down from the spawn spot must not start
/// solid (ignoring the zombie); otherwise no crab is released and the zombie dies whole.
fn headcrab_fits(physics: &Physics, spot: Vec3, npc: usize) -> bool {
    use hl2_simulation::npc_probe::NpcCollisionWorld;
    !physics
        .npc_trace_hull(
            spot,
            spot - Vec3::Z,
            hl2_simulation::ai_hull::HullType::Tiny.bounds(),
            hl2_simulation::npc_probe::Query {
                excluded_entities: &[npc],
                ..Default::default()
            },
        )
        .trace
        .start_solid
}
impl Gameplay {
    /// CNPC_BaseZombie::BecomeTorso for the zombies a blast split this tick (DESIGN 14o
    /// session 37). Source keeps the zombie entity and swaps its model and hull; here a
    /// runtime npc_zombie_torso from its pool template replaces it 40 units up with the
    /// zombie's keyvalues (name, squad, outputs, flags), AI state and burning, half its max
    /// health, falling to the ground; the zombie leaves without dying. The legs are a
    /// raggib (CreateRagGib) at the zombie's origin and angles with the legs force. With
    /// a threshold-1 blast the torso dies at once (BecomeRagdollOnClient with the force).
    pub(super) fn split_zombies(&mut self, physics: &mut Physics) {
        use hl2_simulation::ai::{spawn, zombie};
        let splits = std::mem::take(&mut self.scene.zombie_splits);
        let mut done = std::collections::BTreeSet::new();
        for split in splits {
            let id = split.entity;
            if !done.insert(id) || self.scene.states.get(id).is_none_or(|s| s.killed) {
                continue;
            }
            let Some(&template) = spawn::templates(&self.world, "npc_zombie_torso").first() else {
                self.zombie_split_failures
                    .push(format!("{id}: no npc_zombie_torso template"));
                continue;
            };
            let (origin, rotation) = (self.scene.states[id].origin, self.scene.states[id].rotation);
            let facing = rotation * Vec3::X;
            let yaw = facing.y.atan2(facing.x).to_degrees();
            let keep = self.torso_keyvalues(id);
            let overrides: Vec<(&str, String)> =
                keep.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            let torso = match self.create_from_template(
                template,
                origin + Vec3::Z * zombie::TORSO_LIFT,
                yaw,
                &overrides,
                physics,
            ) {
                Ok(torso) => torso,
                Err(error) => {
                    self.zombie_split_failures.push(format!("{id}: {error}"));
                    continue;
                }
            };
            // The zombie leaves (no OnDeath, no ragdoll: the torso is it from now on).
            let state = &mut self.scene.states[id];
            state.killed = true;
            state.visible = false;
            state.enabled = false;
            physics.set_entity(id, origin, rotation, false);
            let now = self.scene.time;
            // No second handle on the World while the legs are created below: make_mut
            // would copy the whole map (a frame hitch).
            self.ai
                .become_torso(&self.world, &mut self.scene, id, torso);
            self.scene.states[torso].value = split.max_health;
            self.scene.npc_max_health.insert(torso, split.max_health);
            self.torso_fire((id, torso), origin, split.ignite, now);
            let legs = self.split_legs(id, (origin, yaw), split.legs_force, physics);
            if let Some(force) = split.dies {
                let state = &mut self.scene.states[torso];
                state.value = 0.;
                state.killed = true;
                state.visible = false;
                physics.set_entity(torso, origin, rotation, false);
                self.scene.fire(torso, "OnDeath", usize::MAX);
                self.scene.npc_deaths.push(torso);
                self.scene.npc_death_forces.insert(torso, (force, None));
            }
            self.zombie_splits.push((id, torso, legs));
        }
    }
    /// The keyvalues the torso takes from the zombie it replaces. SF_NPC_FALL_TO_GROUND: the
    /// replacement must not take StartNPC's floor drop (the zombie entity natively stays the
    /// same; the torso falls from 40 units up).
    fn torso_keyvalues(&self, id: usize) -> Vec<(String, String)> {
        let mut flags = 0;
        let mut keep = Vec::new();
        for (key, value) in &self.world.entities[id].properties {
            match key.to_ascii_lowercase().as_str() {
                "classname" | "model" | "origin" | "angles" | "body" | "skin" | "health" => {}
                "spawnflags" => flags = value.trim().parse::<u32>().unwrap_or(0),
                _ => keep.push((key.clone(), value.clone())),
            }
        }
        let flags = flags | hl2_simulation::npc_makers::SF_NPC_FALL_TO_GROUND;
        keep.push(("spawnflags".to_owned(), flags.to_string()));
        keep
    }
    /// BecomeTorso: a burning zombie's torso burns 30 s; then 50% of blasts ignite it for 5-10 s
    /// (Ignite while burning does nothing).
    fn torso_fire(
        &mut self,
        (id, torso): (usize, usize),
        origin: Vec3,
        ignite: Option<f32>,
        now: f64,
    ) {
        let flammable = hl2_simulation::fire::Interactions {
            flammable: true,
            ..Default::default()
        };
        let center = origin + Vec3::Z * (hl2_simulation::ai::zombie::TORSO_LIFT + 12.);
        if self.scene.fires.burning(id) {
            self.scene.fires.extinguish(id, origin);
            self.scene
                .fires
                .ignite(torso, flammable, 30., 16., center, now);
        }
        if let Some(time) = ignite {
            self.scene
                .fires
                .ignite(torso, flammable, time, 16., center, now);
        }
    }
    /// The legs raggib of a split zombie at its origin and yaw, with the legs force.
    fn split_legs(
        &mut self,
        id: usize,
        (origin, yaw): (Vec3, f32),
        force: Vec3,
        physics: &mut Physics,
    ) -> Option<usize> {
        use hl2_simulation::ai::spawn;
        let legs = spawn::templates(&self.world, &spawn::gib_label(spawn::GIB_MODELS[0]))
            .first()
            .copied()
            .and_then(|gib| {
                self.create_from_template(gib, origin, yaw, &[], physics)
                    .map_err(|e| self.zombie_split_failures.push(format!("{id} legs: {e}")))
                    .ok()
            })?;
        let world = self.world.clone();
        if !self
            .ragdolls
            .spawn_gib(&world, &mut self.scene, physics, legs, force)
        {
            self.zombie_split_failures
                .push(format!("{id} legs: no ragdoll"));
        }
        Some(legs)
    }
    /// ReleaseHeadcrab(RELEASE_RAGDOLL) at a lethal blast (DESIGN 14o session 37): the crab
    /// as a raggib whose root bone sits on the zombie's "headcrab" attachment
    /// (SetHeadcrabSpawnLocation, roll removed; HeadcrabFits is not checked), pushed with
    /// the damage force x 0.25; the body dies headless (the crabless classic.mdl posed
    /// like the zombie), or DieChopped splits it into legs at the origin and a headless
    /// torso 64 up pitched -90.
    pub(super) fn release_crab(
        &mut self,
        release: hl2_simulation::ai::zombie::CrabRelease,
        physics: &mut Physics,
    ) -> bool {
        let npc = release.entity;
        let (origin, rotation) = (
            self.scene.states[npc].origin,
            self.scene.states[npc].rotation,
        );
        let facing = rotation * Vec3::X;
        let yaw = facing.y.atan2(facing.x).to_degrees();
        // Read the World through the field, never a second Arc handle: an entity created
        // below while another handle lives makes Arc::make_mut copy the whole map (the
        // frame hitch of every crab release before session 38).
        let world = &*self.world;
        let (crab_model, crab_class) =
            hl2_simulation::ai::zombie::headcrab_of(world.entities[npc].class());
        // The poison and fast zombies turn their own crab off and ragdoll themselves.
        let own_body = crab_class != "npc_headcrab";
        let (crab_origin, crab_angles) = self.crab_spot(npc, crab_model, origin, yaw);
        let world = &*self.world;
        // The headless body: the crabless model of the zombie's own kind.
        let corpse_model = if world.entities[npc].class() == "npc_zombie_torso" {
            "models/zombie/classic_torso.mdl"
        } else {
            "models/zombie/classic.mdl"
        };
        if release.live {
            let fits = headcrab_fits(physics, crab_origin, npc);
            if !fits {
                self.zombie_split_failures
                    .push(format!("{npc} crab does not fit"));
                if let Some((legs, torso)) = release.chopped.filter(|_| release.sliced) {
                    self.spawn_chopped(origin, yaw, legs, torso, physics);
                    return true;
                }
                return false;
            }
            self.live_crab(npc, crab_class, (crab_origin, crab_angles.y), physics);
            self.scene.npc_death_forces.insert(npc, (Vec3::ZERO, None));
            self.crab_releases.push((npc, false));
            // fRemoveHead on the zombie itself (runtime bodygroups); its ragdoll follows.
            if own_body {
                self.scene.set_bodygroup(&self.world, npc, 1, 0);
                return false;
            }
            if let Some((legs, torso)) = release.chopped.filter(|_| release.sliced) {
                // DieChopped after its live crab dropped.
                self.spawn_chopped(origin, yaw, legs, torso, physics);
                return true;
            }
            self.spawn_raggib(
                corpse_model,
                origin,
                Vec3::new(0., yaw, 0.),
                Vec3::ZERO,
                Some(npc),
                physics,
            );
            return true;
        }
        // A sliced zombie that kept its crab: it stays on the torso.
        if !release.sliced {
            self.spawn_raggib(
                crab_model,
                crab_origin,
                crab_angles,
                release.crab_force,
                None,
                physics,
            );
        }
        if own_body {
            // The headless zombie ragdolls itself with its death force.
            self.scene.set_bodygroup(&self.world, npc, 1, 0);
            self.crab_releases.push((npc, false));
            return false;
        }
        match release.chopped {
            Some((legs, torso)) => self.spawn_chopped(origin, yaw, legs, torso, physics),
            None => {
                self.spawn_raggib(
                    corpse_model,
                    origin,
                    Vec3::new(0., yaw, 0.),
                    Vec3::ZERO,
                    Some(npc),
                    physics,
                );
            }
        }
        self.crab_releases.push((npc, release.chopped.is_some()));
        true
    }
    /// Where a released crab appears: its root bone on the zombie's "headcrab" attachment
    /// (SetHeadcrabSpawnLocation, roll removed), else EyePosition with the zombie's angles.
    fn crab_spot(&self, npc: usize, crab_model: &str, origin: Vec3, yaw: f32) -> (Vec3, Vec3) {
        // Through the field, never a second Arc handle (see `release_crab`).
        let world = &*self.world;
        let root = hl2_simulation::ai::spawn::templates(
            world,
            &hl2_simulation::ai::spawn::gib_label(crab_model),
        )
        .first()
        .and_then(|t| world.model_instances.iter().find(|i| i.entity == Some(*t)))
        .and_then(|i| world.rigs.get(&i.asset_key()))
        .and_then(|rig| {
            let first = rig.sequences.iter().min_by_key(|s| s.order)?;
            let skin = rig.matrices(&first.name, 0.);
            Some(*skin.first()? * rig.bones.first()?.inverse_bind.inverse())
        });
        let attachment = self.scene.attachment_frames(world, npc, &["headcrab"])[0];
        match (attachment, root) {
            (Some(frame), Some(root)) => {
                let (_, r, t) = (frame * root.inverse()).to_scale_rotation_translation();
                let (y, p, _) = r.to_euler(glam::EulerRot::ZYX);
                (t, Vec3::new(p.to_degrees(), y.to_degrees(), 0.))
            }
            _ => (origin + Vec3::Z * 64., Vec3::new(0., yaw, 0.)),
        }
    }
    /// RELEASE_IMMEDIATE: a live npc_headcrab (no pitch) at this spot and yaw, with the zombie's
    /// squad and inherited flags (gag, long range, fade corpse, always think) plus
    /// SF_NPC_FALL_TO_GROUND; a burning zombie's crab burns 30 s (ShouldIgniteZombieGib).
    fn live_crab(
        &mut self,
        npc: usize,
        crab_class: &str,
        (crab_origin, yaw): (Vec3, f32),
        physics: &mut Physics,
    ) {
        let flags = self.world.entities[npc]
            .get("spawnflags")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(0);
        let flags =
            (flags & (2 | 256 | 512 | 1024)) | hl2_simulation::npc_makers::SF_NPC_FALL_TO_GROUND;
        let mut overrides = vec![("spawnflags", flags.to_string())];
        if let Some(squad) = self.world.entities[npc].get("squadname") {
            overrides.push(("squadname", squad.to_owned()));
        }
        let crab = hl2_simulation::ai::spawn::templates(&self.world, crab_class)
            .first()
            .copied()
            .and_then(|template| {
                self.create_from_template(template, crab_origin, yaw, &overrides, physics)
                    .map_err(|e| self.zombie_split_failures.push(format!("{npc} crab: {e}")))
                    .ok()
            });
        let Some(crab) = crab else {
            return;
        };
        let now = self.scene.time;
        self.ai.released_headcrab(npc, crab, now);
        if self.scene.fires.burning(npc) {
            let flammable = hl2_simulation::fire::Interactions {
                flammable: true,
                ..Default::default()
            };
            self.scene
                .fires
                .ignite(crab, flammable, 30., 8., crab_origin, now);
        }
        self.released_crabs.push(crab);
    }
}
