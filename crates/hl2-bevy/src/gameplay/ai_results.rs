//! What this tick's AI asked of the host: hits on the player, grenades, melee and combine balls.
use super::Gameplay;
use hl2_simulation::physics::Physics;
use modkit_core::movement::Player;

impl Gameplay {
    pub(super) fn ai_player_state(
        &self,
        player: &Player,
        fly: bool,
        facing: glam::Vec3,
    ) -> hl2_simulation::ai::runtime::PlayerState {
        hl2_simulation::ai::runtime::PlayerState {
            facing,
            feet: player.feet,
            velocity: player.velocity,
            crouched: player.crouched,
            on_ground: player.grounded,
            dead: player.dead,
            suit: self.inventory.suit,
            noclip: fly,
        }
    }
    /// This tick's actions. CHL2_Player::PlayerRunCommand: no +USE on a barnacle.
    pub(super) fn take_actions(&mut self) -> Vec<super::Action> {
        let mut actions = std::mem::take(&mut self.actions);
        if self.ai.player_held() {
            actions.retain(|action| !matches!(action, super::Action::Use));
        }
        actions
    }
    /// Props on a barnacle's tongue follow its tip.
    fn move_tongue_props(&mut self, physics: &mut Physics) {
        for (prop, velocity) in std::mem::take(&mut self.ai.prop_moves) {
            if let Some(body) = physics.entity_body(prop) {
                physics.set_body_velocity(body, velocity, glam::Vec3::ZERO);
            }
        }
    }
    pub(super) fn apply_ai_results(&mut self, physics: &mut Physics) {
        self.move_tongue_props(physics);
        for hit in std::mem::take(&mut self.ai.player_hits) {
            if hit.damage > 0. {
                self.scene
                    .player_damage
                    .push(hl2_simulation::player_damage::DamageInfo {
                        amount: hit.damage,
                        kind: hl2_simulation::player_damage::DMG_CLUB,
                        inflictor: self.scene.states[hit.attacker].origin,
                    });
            }
            self.player_impulse += hit.impulse;
            if let Some(fade) = hit.fade {
                self.scene.screen_fades.push(fade);
            }
            // ViewPunch needs the punch-angle path of the arsenal branch (not here).
        }
        // Combine grenade tosses: Fraggrenade_Create (NPC damage, 3.5 s fuse; the soldier
        // is the thrower and the blast's attacker).
        for (thrower, position, velocity, spin) in std::mem::take(&mut self.ai.grenade_tosses) {
            let damage = self.scene.skill.get("sk_npc_dmg_fraggrenade", 75.);
            let radius = self.scene.skill.get("sk_fraggrenade_radius", 250.);
            let at = self.scene.time;
            self.projectiles.spawn_npc(
                hl2_simulation::projectiles::ProjectileSpawn {
                    angles: Some(glam::Vec3::ZERO),
                    kind: hl2_simulation::projectiles::ProjectileKind::FragGrenade,
                    position,
                    velocity,
                    angular_velocity: spin,
                    at,
                    damage,
                    radius,
                    mass: 0.,
                    lifetime: 3.5,
                },
                thrower,
                &mut self.scene,
            );
        }
        // NPC melee (stunstick swings on hated NPCs): DMG_CLUB through the shared hit path.
        for melee in std::mem::take(&mut self.ai.melee_hits) {
            let mut blood = Vec::new();
            let origin = self.scene.states[melee.attacker].origin + glam::Vec3::Z * 36.;
            hl2_simulation::gameplay::apply_hit(
                &self.world,
                &mut self.scene,
                physics,
                melee.damage,
                "",
                &hl2_simulation::physics::RayHit {
                    entity: melee.victim,
                    position: melee.position,
                    normal: -melee.direction,
                    hitgroup: 0,
                    bone: None,
                },
                melee.direction,
                true,
                hl2_simulation::gameplay::HitSource {
                    player: false,
                    npc: Some(melee.attacker),
                    origin,
                },
                &mut || self.ai.uniform(),
                &mut blood,
            );
            self.ai.npc_attacked(melee.victim, melee.attacker);
            for (hit, color) in blood {
                self.impacts
                    .add_blood(hit, color, &self.world, physics, &self.scene);
            }
        }
        // Elite combine balls (CreateCombineBall: sk_weapon_ar2_alt_fire_radius/mass/
        // duration; the AR2's own ball damage).
        for (owner, position, velocity) in self.ai.take_ball_launches() {
            let Some(ar2) = self.weapons.get("weapon_ar2") else {
                continue;
            };
            let launch = hl2_simulation::projectiles::ProjectileSpawn {
                angles: None,
                kind: hl2_simulation::projectiles::ProjectileKind::CombineBall,
                position,
                velocity,
                angular_velocity: glam::Vec3::ZERO,
                at: self.scene.time,
                damage: ar2.secondary_damage,
                radius: ar2.secondary_radius,
                mass: ar2.secondary_mass,
                lifetime: ar2.secondary_lifetime,
            };
            self.projectiles
                .spawn_npc_ball(launch, owner, &mut self.scene);
        }
    }
}
