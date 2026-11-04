//! What a dying NPC leaves behind: a poison zombie's nest crabs and a dropped weapon.
use super::Gameplay;
use glam::Vec3;
use hl2_simulation::physics::Physics;

impl Gameplay {
    /// CNPC_PoisonZombie::EvacuateNest (DESIGN 14o session 38): the crab in its hand only
    /// loses its bodygroup (the SDK adds it to the count but ejects nest slots only); each
    /// nest crab becomes a npc_headcrab_poison at its headcrab2/3/4 attachment with a random
    /// yaw (HeadcrabFits, or it is dropped), burning 100 s with a burning zombie, ejected at
    /// RandomInt(20, 50) x (U(-1,1), U(-1,1), U(0.5,1)) x 2 (0.1 x the blast damage for a
    /// blast over 10), and its nest bodygroup turns off on the corpse.
    pub(super) fn evacuate_nest(&mut self, zombie: usize, physics: &mut Physics) {
        use hl2_simulation::ai::zombie;
        let Some((nest, holding, enemy)) = self.ai.take_poison_nest(zombie) else {
            return;
        };
        if holding {
            self.scene
                .set_bodygroup(&self.world, zombie, zombie::BODYGROUP_THROW, 0);
        }
        let scale = match self.scene.npc_death_blasts.get(&zombie) {
            Some(&damage) if damage > 10. => 0.1 * damage,
            _ => 2.,
        };
        let names = ["headcrab2", "headcrab3", "headcrab4"];
        let frames = self.scene.attachment_frames(&self.world, zombie, &names);
        let time = self.scene.time;
        for (i, frame) in frames.into_iter().enumerate() {
            if !nest[i] {
                continue;
            }
            let roll = |salt| hl2_simulation::fire::roll(zombie, time, 60 + i as u64 * 8 + salt);
            let origin = frame.map_or(self.scene.states[zombie].origin + Vec3::Z * 50., |f| {
                f.w_axis.truncate()
            });
            let yaw = 360. * roll(0);
            {
                use hl2_simulation::npc_probe::NpcCollisionWorld;
                // HeadcrabFits ignores only the new crab and the zombie: a crab ejected
                // a moment ago (or the released head crab) blocks an overlapping nest
                // spot, so the later crab is removed (native blast kills mostly leave
                // one). Those crabs are not in the collision queries yet.
                let tiny = hl2_simulation::ai_hull::HullType::Tiny.bounds();
                let (lo, hi) = (origin + tiny.mins - Vec3::Z, origin + tiny.maxs);
                let blocked = self.world.entities.iter().enumerate().any(|(id, e)| {
                    id != zombie
                        && e.class().starts_with("npc_")
                        && self.scene.states.get(id).is_some_and(|s| !s.killed)
                        && hl2_simulation::ai_hull::class_hull(e.class()).is_some_and(|h| {
                            let b = h.bounds();
                            let at = self.scene.states[id].origin;
                            let (a, z) = (at + b.mins, at + b.maxs);
                            a.cmplt(hi).all() && z.cmpgt(lo).all()
                        })
                });
                let fits = !blocked
                    && !physics
                        .npc_trace_hull(
                            origin,
                            origin - Vec3::Z,
                            hl2_simulation::ai_hull::HullType::Tiny.bounds(),
                            hl2_simulation::npc_probe::Query {
                                excluded_entities: &[zombie],
                                ..Default::default()
                            },
                        )
                        .trace
                        .start_solid;
                if !fits {
                    self.zombie_split_failures
                        .push(format!("{zombie} nest crab {i} does not fit"));
                    continue;
                }
            }
            let Some(template) = self.black_crab_template() else {
                continue;
            };
            let flags = hl2_simulation::npc_makers::SF_NPC_FALL_TO_GROUND;
            let crab = match self.create_from_template(
                template,
                origin,
                yaw,
                &[("spawnflags", flags.to_string())],
                physics,
            ) {
                Ok(crab) => crab,
                Err(e) => {
                    self.zombie_split_failures
                        .push(format!("{zombie} nest crab: {e}"));
                    continue;
                }
            };
            if self.scene.fires.burning(zombie) {
                let flammable = hl2_simulation::fire::Interactions {
                    flammable: true,
                    ..Default::default()
                };
                self.scene
                    .fires
                    .ignite(crab, flammable, 100., 8., origin, time);
            }
            let speed = scale * (20. + (31. * roll(1)).floor().min(30.));
            let velocity =
                speed * Vec3::new(2. * roll(2) - 1., 2. * roll(3) - 1., 0.5 + 0.5 * roll(4));
            self.ai
                .ejected_headcrab(zombie, crab, velocity, enemy, &mut self.scene);
            self.scene
                .set_bodygroup(&self.world, zombie, zombie::BODYGROUP_NEST_BASE + i, 0);
            self.evacuated_crabs.push(crab);
        }
    }
    /// One killed NPC's active weapon as a new VPhysics weapon entity.
    pub(super) fn drop_weapon(&mut self, npc: usize, physics: &mut Physics) {
        let Some(class) = self.ai.weapon_class(npc) else {
            return;
        };
        let Some(model) = self.ai.weapon_model(&class) else {
            return;
        };
        let flags = self.world.entities[npc]
            .get("spawnflags")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .unwrap_or(0);
        if flags & (1 << 13) != 0 || !self.world.model_physics.contains_key(&format!("{model}#0")) {
            return;
        }
        let state = &self.scene.states[npc];
        let body = state.rotation * Vec3::X;
        let flat = Vec3::new(body.x, body.y, 0.).normalize_or(Vec3::X);
        // Weapon_Drop teleports a bone-merged weapon to the owner's Weapon_bone pose
        // (times its inverse root bone); without a rig, in front of the shoot position.
        let (origin, angles) = self
            .scene
            .merged_model_frame(&self.world, npc, &format!("{model}#0"))
            .map(|frame| {
                let (_, rotation, translation) = frame.to_scale_rotation_translation();
                let (yaw, pitch, roll) = rotation.to_euler(glam::EulerRot::ZYX);
                (
                    translation,
                    Vec3::new(pitch.to_degrees(), yaw.to_degrees(), roll.to_degrees()),
                )
            })
            .unwrap_or_else(|| {
                let yaw = flat.y.atan2(flat.x).to_degrees();
                (
                    state.origin + Vec3::Z * 55. + flat * 10.,
                    Vec3::new(0., yaw, 0.),
                )
            });
        let clip = if class == "weapon_smg1" {
            let health =
                hl2_simulation::gameplay::npc_spawn_health(&self.scene, &self.world.entities[npc]);
            let shot = self.scene.skill.get("sk_plr_dmg_smg1", 4.)
                * self.scene.skill.player_inflict_scale();
            ((health / shot.max(0.01)) * 2.) as i32
        } else {
            self.weapons.get(&class).map_or(0, |w| w.default_clip)
        };
        let id = self.world.entities.len();
        let entity = modkit_core::Entity {
            properties: [
                ("classname", class.clone()),
                ("origin", format!("{} {} {}", origin.x, origin.y, origin.z)),
                ("angles", format!("{} {} {}", angles.x, angles.y, angles.z)),
                ("model", model.clone()),
                ("ammo", clip.to_string()),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v))
            .collect(),
        };
        let instance = modkit_core::ModelInstance {
            background: false,
            model: model.clone(),
            origin,
            angles,
            skin: 0,
            body: 0,
            scale: 1.,
            // A VPhysics object like a physics prop (its .phy and propdata).
            kind: "prop_physics".into(),
            solid: true,
            solid_mode: None,
            entity: Some(id),
        };
        let Ok(id) = self.insert_entity(entity, Some(instance), physics) else {
            return;
        };
        // FallInit: COLLISION_GROUP_WEAPON (the player and NPCs pass over it).
        physics.set_weapon_group(id);
        let speed = 64. + (npc.wrapping_mul(2_654_435_761) % 65) as f32;
        if let Some(handle) = physics.entity_body(id) {
            physics.set_body_velocity(handle, body * speed, Vec3::splat(200.));
        }
        self.model_draws.push(id);
        self.spawn_log
            .push(serde_json::json!({"weapon_drop":class,"npc":npc,"entity":id,"ammo":clip,"origin":origin.to_array(),"angles":angles.to_array(),"npc_origin":self.scene.states[npc].origin.to_array()}));
    }
}
