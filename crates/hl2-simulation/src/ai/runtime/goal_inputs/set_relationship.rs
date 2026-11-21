//! The NPC input `SetRelationship` (Source SDK `game/server/ai_basenpc.cpp`,
//! `CAI_BaseNPC::AddRelationship`): `"<who> <D_HT|D_FR|D_LI|D_NU> [priority]"`, repeated. `<who>`
//! names entities (each gets an entity relationship), else `player` / `!player`, else a classname
//! (a class relationship, so NPCs of that class spawned later are covered too). Priorities are
//! read but not ranked yet: the overrides keep only the disposition.

use super::super::{AiRuntime, Disposition, PLAYER};
use crate::ai::relationships::{classify, Class};
use modkit_core::World;

/// One `who disposition [priority]` entry.
#[derive(Debug, PartialEq)]
pub(super) struct Entry<'a> {
    pub who: &'a str,
    pub disposition: Disposition,
}

/// The entries of a `SetRelationship` parameter. Each entry takes three words (the last may
/// lack its priority); an unknown disposition stops the parse, as in the SDK.
pub(super) fn parse(parameter: &str) -> Vec<Entry<'_>> {
    let words: Vec<&str> = parameter.split_whitespace().collect();
    let mut entries = Vec::new();
    for chunk in words.chunks(3) {
        let [who, disposition, ..] = chunk else {
            break;
        };
        let disposition = match disposition.to_ascii_uppercase().as_str() {
            "D_HT" => Disposition::HT,
            "D_FR" => Disposition::FR,
            "D_LI" => Disposition::LI,
            "D_NU" => Disposition::NU,
            _ => break,
        };
        entries.push(Entry { who, disposition });
    }
    entries
}

impl AiRuntime {
    /// `SetRelationship` on NPC `subject`.
    pub(super) fn set_relationship(&mut self, world: &World, subject: usize, parameter: &str) {
        for Entry { who, disposition } in parse(parameter) {
            let named: Vec<usize> = world
                .entities
                .iter()
                .enumerate()
                .filter(|(id, e)| {
                    *id != subject
                        && e.get("targetname")
                            .is_some_and(|n| n.eq_ignore_ascii_case(who))
                })
                .map(|(id, _)| id)
                .collect();
            if !named.is_empty() {
                for target in named {
                    self.relationship_overrides
                        .insert((subject, target), disposition);
                }
            } else if who.eq_ignore_ascii_case("player") || who.eq_ignore_ascii_case("!player") {
                self.relationship_overrides
                    .insert((subject, PLAYER), disposition);
            } else {
                let class = classify(&who.to_ascii_lowercase(), false);
                if class != Class::None {
                    self.applied_relationships
                        .class_overrides
                        .insert((subject, class), disposition);
                }
            }
        }
    }

    /// A class relationship `SetRelationship` gave `subject` toward `target`'s class.
    pub(in crate::ai::runtime) fn class_override(
        &self,
        subject: usize,
        target: Class,
    ) -> Option<Disposition> {
        self.applied_relationships
            .class_overrides
            .get(&(subject, target))
            .copied()
    }
}

#[cfg(test)]
mod tests {
    use super::{parse, Entry};
    use crate::ai::relationships::Disposition;

    #[test]
    fn entries_take_three_words_and_stop_at_an_unknown_disposition() {
        assert_eq!(
            parse("player D_LI 99 npc_metropolice d_ht 50 rebel"),
            [
                Entry {
                    who: "player",
                    disposition: Disposition::LI
                },
                Entry {
                    who: "npc_metropolice",
                    disposition: Disposition::HT
                },
            ]
        );
        assert_eq!(
            parse("player D_NU"),
            [Entry {
                who: "player",
                disposition: Disposition::NU
            }]
        );
        assert!(parse("player D_XX 99 npc_zombie D_HT 1").is_empty());
    }

    #[test]
    fn a_manhack_told_to_like_the_player_and_hate_cops_does_so() {
        use super::super::super::{AiRuntime, PLAYER};
        let entity = |pairs: &[(&str, &str)]| modkit_core::Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        };
        let world = modkit_core::World {
            entities: vec![
                entity(&[("classname", "npc_manhack"), ("origin", "0 0 64")]),
                entity(&[("classname", "npc_metropolice"), ("origin", "300 0 0")]),
                entity(&[
                    ("classname", "npc_metropolice"),
                    ("targetname", "cop2"),
                    ("origin", "600 0 0"),
                ]),
            ],
            ..Default::default()
        };
        let mut ai = AiRuntime::new(&world, None);
        let before = ai.npc_disposition(&world, 1, 0, false);
        ai.set_relationship(&world, 0, "player D_LI 99 npc_metropolice D_HT 99");
        assert_eq!(
            ai.relationship_overrides.get(&(0, PLAYER)),
            Some(&Disposition::LI)
        );
        assert_eq!(ai.npc_disposition(&world, 0, 1, false), Disposition::HT);
        assert_eq!(ai.npc_disposition(&world, 0, 2, false), Disposition::HT);
        assert_eq!(
            ai.npc_disposition(&world, 1, 0, false),
            before,
            "one-sided: the cop's view of the manhack is unchanged"
        );
        ai.set_relationship(&world, 0, "cop2 D_NU");
        assert_eq!(
            ai.npc_disposition(&world, 0, 2, false),
            Disposition::NU,
            "a name beats the class"
        );
    }
}
