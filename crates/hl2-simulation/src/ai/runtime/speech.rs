//! Speech: idle chatter, ally lines, sentences and pain/death sounds.

use super::{
    humanoid, sentence_criteria, sentence_priority, speech_group, AiActor, AiClass, AiRuntime,
    Criteria, IdleThen, NpcState, PlayerState, Speech, COP_CHATTER_ASK_QUESTION,
    COP_CHATTER_RESPONSE, COP_CHATTER_WAIT_FOR_RESPONSE, PLAYER, SF_NPC_GAG,
};
use crate::entities::Scene;
use glam::Vec3;
use modkit_core::World;

impl AiRuntime {
    /// IdleSound's lines, and their effects when Speak succeeds.
    pub(super) fn speak_idle_lines(&mut self, world: &World, scene: &mut Scene) {
        for line in std::mem::take(&mut self.idle_lines) {
            if !self.say(line.id, line.group, world, scene) {
                continue;
            }
            let squadmates: Vec<usize> = self.actors[&line.id]
                .squad
                .as_ref()
                .and_then(|n| self.squads.get(n))
                .map_or(Vec::new(), |s| {
                    s.members
                        .iter()
                        .copied()
                        .filter(|m| *m != line.id)
                        .collect()
                });
            let mut set = |id: usize, f: &mut dyn FnMut(&mut AiActor)| {
                if let Some(actor) = self.actors.get_mut(&id) {
                    f(actor);
                }
            };
            match line.then {
                IdleThen::Nothing => {}
                IdleThen::CombineQuestion(q) => self.combine_question = q,
                IdleThen::CopHarassed => {
                    for m in std::iter::once(line.id).chain(squadmates) {
                        set(m, &mut |a| a.harassed = true);
                    }
                }
                IdleThen::CopAsked(question) => {
                    set(line.id, &mut |a| {
                        a.idle_chatter = COP_CHATTER_WAIT_FOR_RESPONSE
                    });
                    for m in squadmates {
                        set(m, &mut |a| a.idle_chatter = COP_CHATTER_RESPONSE + question);
                    }
                }
                IdleThen::CopAnswered => {
                    for m in std::iter::once(line.id).chain(squadmates) {
                        set(m, &mut |a| a.idle_chatter = COP_CHATTER_ASK_QUESTION);
                    }
                }
            }
        }
    }

    /// CAI_SentenceBase::Speak: FOkToMakeSound (no squads yet) culls a sentence of equal
    /// or lower priority while the last one's timer runs; JustMadeSound(priority, 2 s)
    /// sets it to 2 + U(1.5, 2.0) s. False when culled.
    /// The sound cues ally speech can play (audio preload).
    pub fn speech_cues(&self) -> Vec<String> {
        self.ally.cues()
    }

    /// CAI_PlayerAlly::SpeakIfAllowed(concept): the talker rules pick a response for this
    /// NPC's criteria; its sound plays at the speaker (lip sync through the actor).
    pub(super) fn ally_say(
        &mut self,
        id: usize,
        concept: &'static str,
        world: &World,
        scene: &mut Scene,
        player: PlayerState,
    ) -> bool {
        let Some(e) = world.entities.get(id) else {
            return false;
        };
        let Some(actor) = self.actors.get(&id) else {
            return false;
        };
        let state = &scene.states[id];
        let model = world
            .model_instances
            .iter()
            .find(|i| i.entity == Some(id))
            .map_or("", |i| i.model.as_str());
        let enemy = actor.npc.enemy.map(|en| {
            if en == PLAYER {
                "player"
            } else {
                world.entities.get(en).map_or("", |x| x.class())
            }
        });
        let npc_state = format!("{:?}", actor.npc.state);
        let weapon = actor.weapon.as_ref().map(|w| w.class.clone());
        let gagged = e
            .get("spawnflags")
            .and_then(|v| v.trim().parse::<u32>().ok())
            .is_some_and(|f| f & SF_NPC_GAG != 0);
        let speaker = crate::ai::ally_speech::Speaker {
            id,
            classname: e.class(),
            name: e.get("targetname").unwrap_or(""),
            // An NPC nobody hurt yet keeps 0 here and has its spawn health.
            health: if state.killed {
                0.
            } else if state.value > 0. {
                state.value
            } else {
                crate::gameplay::npc_spawn_health(scene, e)
            },
            max_health: crate::gameplay::npc_spawn_health(scene, e),
            npc_state: &npc_state,
            enemy,
            weapon: weapon.as_deref(),
            // sv_npc_talker_maxdist measures to the player whether or not he flies.
            distance_to_player: (state.origin + Vec3::Z * 36.).distance(player.center()),
            map: &world.name,
            gender: if model.to_ascii_lowercase().contains("female") {
                "female"
            } else {
                "male"
            },
            gagged,
            now: scene.time,
        };
        let mut rolls = [0f32; 8];
        for r in &mut rolls {
            *r = self.random();
        }
        let mut next = rolls.into_iter().cycle();
        let mut random = move || next.next().unwrap_or(0.5);
        let sound = match self.ally.speak(concept, &speaker, &mut random) {
            Ok(sound) => {
                *self
                    .diagnostics
                    .ally_speech
                    .entry(format!("{concept}:spoken"))
                    .or_default() += 1;
                sound
            }
            Err(reason) => {
                *self
                    .diagnostics
                    .ally_speech
                    .entry(format!("{concept}:{reason}"))
                    .or_default() += 1;
                return false;
            }
        };
        let origin = state.origin;
        scene.sounds.push(crate::sounds::SoundRequest {
            origin: Some(origin + Vec3::Z * 64.),
            // GenderExpandString needs the speaker's model: without it a `$gender`
            // wave picks from both genders (owner report: female citizens said male
            // pain lines and the reverse).
            actor: Some(crate::sounds::SoundActor {
                name: e.get("targetname").unwrap_or("").into(),
                model: model.to_owned(),
                entity: Some(id),
            }),
            ..sound.into()
        });
        true
    }

    /// CAI_SentenceBase::Speak: MatchesCriteria (the group's SentenceCriteria_t), then a
    /// prioritized line asks FOkToMakeSound (the squad's gate in a squad, else the NPC's)
    /// and JustMadeSound(priority, 2 s) sets the NPC's and the squad's waits to
    /// 2 + U(1.5, 2.0) s. False when refused or culled.
    pub(super) fn say(
        &mut self,
        id: usize,
        group: &'static str,
        world: &World,
        scene: &mut Scene,
    ) -> bool {
        let origin = scene.states[id].origin;
        let Some(actor) = self.actors.get(&id) else {
            return false;
        };
        let squad = actor.squad.as_ref().and_then(|name| self.squads.get(name));
        let gagged = actor.npc.state != NpcState::Combat
            && world.entities.get(id).is_some_and(|e| {
                e.get("spawnflags")
                    .and_then(|v| v.trim().parse::<u32>().ok())
                    .is_some_and(|f| f & SF_NPC_GAG != 0)
            });
        let matches = match sentence_criteria(group) {
            Criteria::Always => true,
            Criteria::Normal => !gagged,
            Criteria::InSquad => !gagged && squad.is_some_and(|s| s.num_members() > 1),
            Criteria::SquadLeader => {
                !gagged && squad.is_some_and(|s| s.num_members() > 1 && s.leader() == Some(id))
            }
        };
        if !matches {
            *self
                .diagnostics
                .criteria_refused
                .entry(group.into())
                .or_default() += 1;
            return false;
        }
        let squad_name = actor.squad.clone();
        let priority = sentence_priority(group);
        if priority >= 0 {
            let now = scene.time;
            let ok = match squad_name.as_ref().and_then(|n| self.squads.get(n)) {
                Some(squad) => squad.ok_to_make_sound(priority, now),
                None => self
                    .sound_wait
                    .get(&id)
                    .is_none_or(|&(until, last)| now > until || priority > last),
            };
            if !ok {
                self.diagnostics.culled_sentences += 1;
                return false;
            }
            let wait = now + 2. + 1.5 + 0.5 * f64::from(self.random());
            self.sound_wait.insert(id, (wait, priority));
            let squad_wait = now + 2. + 1.5 + 0.5 * f64::from(self.random());
            if let Some(squad) = squad_name.as_ref().and_then(|n| self.squads.get_mut(n)) {
                squad.just_made_sound(priority, squad_wait);
            }
        }
        self.diagnostics.sentences += 1;
        *self
            .diagnostics
            .sentence_groups
            .entry(group.into())
            .or_default() += 1;
        scene.sounds.push(crate::sounds::SoundRequest {
            origin: Some(origin + Vec3::Z * 64.),
            actor: Some(crate::sounds::SoundActor {
                name: scene
                    .entity(world, id)
                    .and_then(|e| e.get("targetname"))
                    .unwrap_or("")
                    .into(),
                model: String::new(),
                entity: Some(id),
            }),
            ..format!("#{group}").into()
        });
        true
    }

    /// CAI_BaseNPC::OnTakeDamage_Alive -> PainSound, or Event_Killed -> DeathSound
    /// (CNPC_MetroPolice/CNPC_Combine): pain at most once a second, the first light and
    /// heavy hits with their own groups; the death line once.
    pub(super) fn pain_or_death_sound(
        &mut self,
        id: usize,
        world: &World,
        scene: &mut Scene,
        player: PlayerState,
    ) {
        let now = scene.time;
        let Some(e) = world.entities.get(id) else {
            return;
        };
        let max = crate::gameplay::npc_spawn_health(scene, e).max(1.);
        let Some(actor) = self.actors.get_mut(&id) else {
            return;
        };
        let state = &scene.states[id];
        // CZombie / CNPC_PoisonZombie PainSound and DeathSound.
        if let AiClass::Zombie(z) = &actor.class {
            let kind = z.kind;
            let cue = if state.killed {
                if std::mem::replace(&mut actor.death_spoken, true) {
                    return;
                }
                kind.cue("Zombie.Die")
            } else {
                if now < actor.next_pain_sound || state.value <= 0. {
                    return;
                }
                actor.next_pain_sound = now + 1.;
                kind.cue("Zombie.Pain")
            };
            let origin = state.origin;
            scene.sounds.push(crate::sounds::SoundRequest {
                origin: Some(origin + Vec3::Z * 64.),
                ..cue.into()
            });
            return;
        }
        // CAI_PlayerAlly::PainSound speaks TLK_WOUND; the companions' DeathSound emits
        // their die sound (CNPC_Citizen/CNPC_Barney/CNPC_Alyx).
        if matches!(&actor.class, AiClass::Humanoid(h) if matches!(h.kind, humanoid::Kind::Companion { .. }))
        {
            if state.killed {
                if std::mem::replace(&mut actor.death_spoken, true) {
                    return;
                }
                let die = match e.class() {
                    "npc_barney" => "npc_barney.die",
                    "npc_alyx" => "npc_alyx.die",
                    _ => "NPC_Citizen.Die",
                };
                let origin = state.origin;
                // EmitSound from the NPC: GenderExpandString reads its model (the
                // citizens' die waves are vo/npc/$gender01/pain*.wav; owner report: dying
                // citizens moaned in the other gender's voice).
                let model = world
                    .model_instances
                    .iter()
                    .find(|i| i.entity == Some(id))
                    .map_or("", |i| i.model.as_str());
                scene.sounds.push(crate::sounds::SoundRequest {
                    origin: Some(origin + Vec3::Z * 64.),
                    actor: Some(crate::sounds::SoundActor {
                        name: e.get("targetname").unwrap_or("").into(),
                        model: model.to_owned(),
                        entity: Some(id),
                    }),
                    ..die.into()
                });
                return;
            }
            if now < actor.next_pain_sound || state.value <= 0. {
                return;
            }
            actor.next_pain_sound = now + 1.;
            self.ally_say(id, "TLK_WOUND", world, scene, player);
            return;
        }
        let speech = if state.killed {
            if std::mem::replace(&mut actor.death_spoken, true) {
                return;
            }
            Speech::Die
        } else {
            let ratio = state.value / max;
            if now < actor.next_pain_sound || ratio <= 0. {
                return;
            }
            actor.next_pain_sound = now + 1.;
            let combine =
                matches!(&actor.class, AiClass::Humanoid(h) if h.kind == humanoid::Kind::Combine);
            if combine {
                if actor.pain_memory & 1 == 0 && ratio > 0.9 {
                    actor.pain_memory |= 1;
                    Speech::PainLight
                } else if actor.pain_memory & 2 == 0 && ratio < 0.25 {
                    actor.pain_memory |= 2;
                    Speech::PainHeavy
                } else {
                    Speech::Pain
                }
            } else if actor.pain_memory & 2 == 0 && ratio < 0.25 {
                actor.pain_memory |= 3;
                Speech::PainHeavy
            } else if actor.pain_memory & 1 == 0 && ratio > 0.8 {
                actor.pain_memory |= 1;
                Speech::PainLight
            } else {
                Speech::Pain
            }
        };
        if let Some(group) = speech_group(&actor.class, speech) {
            self.say(id, group, world, scene);
        }
    }
}
