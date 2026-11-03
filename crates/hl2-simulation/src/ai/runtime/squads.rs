//! Squads: membership, leaving and the squad events members share.

use super::{
    humanoid, AiClass, AiRuntime, SquadEvent, COND_SEE_ENEMY, COP_CHATTER_ASK_QUESTION,
    COP_CHATTER_WAIT_FOR_RESPONSE,
};
use crate::entities::Scene;
use modkit_core::World;

impl AiRuntime {
    /// CAI_BaseNPC::InitSquad for NPCs in the world (map NPCs on the first tick, maker
    /// and runtime copies when they appear; hidden templates never join), and
    /// Event_Killed's RemoveFromSquad(death) -> NotifyDeadFriend for the killed.
    pub(super) fn update_squads(&mut self, world: &World, scene: &mut Scene) {
        let ids: Vec<usize> = self.actors.keys().copied().collect();
        for id in ids {
            let state = &scene.states[id];
            let actor = &self.actors[&id];
            if state.killed {
                if let Some(name) = actor.squad.clone() {
                    self.remove_from_squad(id, &name, true, world, scene);
                }
                continue;
            }
            if actor.squad.is_none() && state.visible {
                if let Some(name) = actor.squad_name.clone() {
                    self.add_to_squad(id, &name, scene.time);
                }
            }
        }
    }

    /// CAI_Squad::AddToSquad: join in order; a later member learns the first member's
    /// enemies (UpdateEnemyMemory informed by it, not first-hand).
    fn add_to_squad(&mut self, id: usize, name: &str, now: f64) {
        let squad = self
            .squads
            .entry(name.to_owned())
            .or_insert_with(|| crate::ai::squad::Squad::new(name));
        squad.add(id);
        let first = squad.members[0];
        *self
            .diagnostics
            .squad_events
            .entry("join".into())
            .or_default() += 1;
        let known = if first == id {
            Vec::new()
        } else {
            self.actors[&first].npc.enemies.known()
        };
        let actor = self.actors.get_mut(&id).expect("actor");
        actor.squad = Some(name.to_owned());
        for (enemy, position) in known {
            actor
                .npc
                .enemies
                .update(Some(enemy), position, 0., false, now);
        }
    }

    /// CAI_Squad::RemoveFromSquad; on death the remaining members get NotifyDeadFriend.
    fn remove_from_squad(
        &mut self,
        id: usize,
        name: &str,
        death: bool,
        world: &World,
        scene: &mut Scene,
    ) {
        let Some(squad) = self.squads.get_mut(name) else {
            return;
        };
        squad.remove(id);
        let members = squad.members.clone();
        let remaining = squad.num_members();
        if let Some(actor) = self.actors.get_mut(&id) {
            actor.squad = None;
            actor.squad_name = None;
        }
        if !death {
            return;
        }
        *self
            .diagnostics
            .squad_events
            .entry("death".into())
            .or_default() += 1;
        let friend_class = world.entities.get(id).map_or("", |e| e.class());
        for member in members {
            if scene.states.get(member).is_none_or(|s| s.killed) {
                continue;
            }
            let Some(actor) = self.actors.get_mut(&member) else {
                continue;
            };
            // CNPC_MetroPolice::NotifyDeadFriend resets idle chatter waiting for an answer.
            if actor.class.cop().is_some()
                && friend_class != "npc_manhack"
                && actor.idle_chatter == COP_CHATTER_WAIT_FOR_RESPONSE
            {
                actor.idle_chatter = COP_CHATTER_ASK_QUESTION;
            }
            // NotifyDeadFriend: CNPC_MetroPolice (no line for a dead manhack) and
            // CNPC_Combine; LAST_OF_SQUAD when fewer than two remain.
            let group = match &actor.class {
                AiClass::Metropolice(_) if friend_class != "npc_manhack" => {
                    Some(if remaining < 2 {
                        "METROPOLICE_LAST_OF_SQUAD"
                    } else {
                        "METROPOLICE_MAN_DOWN"
                    })
                }
                AiClass::Humanoid(h) if h.kind == humanoid::Kind::Combine => {
                    Some(if remaining < 2 {
                        "COMBINE_LAST_OF_SQUAD"
                    } else {
                        "COMBINE_MAN_DOWN"
                    })
                }
                _ => None,
            };
            if let Some(group) = group {
                self.say(member, group, world, scene);
            }
        }
    }

    /// Squad calls made by the last think: shared first-hand sightings
    /// (CAI_Squad::UpdateEnemyMemory) and SquadNewEnemy (a member without an enemy, or
    /// whose other enemy is unseen for over 3 s, takes the new one).
    pub(super) fn apply_squad_events(&mut self, scene: &Scene) {
        let now = scene.time;
        for event in std::mem::take(&mut self.squad_events) {
            let (name, from) = match &event {
                SquadEvent::Memory { squad, from, .. } => (squad, Some(*from)),
                SquadEvent::NewEnemy { squad, .. } => (squad, None),
            };
            let Some(squad) = self.squads.get(name) else {
                continue;
            };
            let members: Vec<usize> = squad.members.clone();
            for member in members {
                if Some(member) == from || scene.states.get(member).is_none_or(|s| s.killed) {
                    continue;
                }
                let Some(actor) = self.actors.get_mut(&member) else {
                    continue;
                };
                match &event {
                    SquadEvent::Memory {
                        enemy, position, ..
                    } => {
                        actor
                            .npc
                            .enemies
                            .update(Some(*enemy), *position, 0., false, now);
                        *self
                            .diagnostics
                            .squad_events
                            .entry("memory".into())
                            .or_default() += 1;
                    }
                    SquadEvent::NewEnemy { enemy, .. } => {
                        let current = actor.npc.enemy;
                        if crate::ai::squad::should_take_squad_enemy(
                            current,
                            *enemy,
                            actor.npc.conditions.has(COND_SEE_ENEMY),
                            now - actor.npc.enemies.last_time_seen(current),
                        ) {
                            actor.npc.enemy = Some(*enemy);
                            actor.squad_new_enemy = true;
                            *self
                                .diagnostics
                                .squad_events
                                .entry("new_enemy".into())
                                .or_default() += 1;
                        }
                    }
                }
            }
        }
    }
}
