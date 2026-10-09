//! Global entity states: the SDK globalstate table and CEnvGlobal (logicentities.cpp).
//! The table survives level changes and is cleared by a new game; env_global with
//! SF_GLOBAL_SET only adds its initial state when the name is not in the table yet.
//! Names are matched without case.
use modkit_core::{Entity, World};
use serde::Serialize;
use std::collections::BTreeMap;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
pub enum GlobalState {
    Off,
    On,
    Dead,
}
impl GlobalState {
    fn from_keyvalue(value: i32) -> Self {
        match value {
            1 => Self::On,
            2 => Self::Dead,
            _ => Self::Off,
        }
    }
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct Globals {
    table: BTreeMap<String, (GlobalState, i32)>,
}
impl Globals {
    /// CEnvGlobal::Spawn for every env_global of a freshly loaded map.
    pub fn spawn(world: &World) -> Self {
        let mut globals = Self::default();
        for e in world.entities.iter().filter(|e| e.class() == "env_global") {
            let Some(name) = Self::name(e) else {
                continue;
            };
            let flags = int(e, "spawnflags");
            if flags & 1 != 0 {
                let initial = GlobalState::from_keyvalue(int(e, "initialstate"));
                globals.table.entry(name.clone()).or_insert((initial, 0));
                let counter = int(e, "counter");
                if counter != 0 {
                    globals.table.get_mut(&name).expect("added").1 = counter;
                }
            }
        }
        globals
    }
    fn name(e: &Entity) -> Option<String> {
        e.get("globalstate")
            .filter(|s| !s.is_empty())
            .map(str::to_ascii_lowercase)
    }
    /// GlobalEntity_GetState: names not in the table are off.
    pub fn state(&self, name: &str) -> GlobalState {
        self.table
            .get(&name.to_ascii_lowercase())
            .map_or(GlobalState::Off, |g| g.0)
    }
    pub fn is_on(&self, name: &str) -> bool {
        self.state(name) == GlobalState::On
    }
    pub fn counter(&self, name: &str) -> i32 {
        self.table
            .get(&name.to_ascii_lowercase())
            .map_or(0, |g| g.1)
    }
    /// A level change keeps the previous table; the new map's SF_GLOBAL_SET entries only
    /// fill names the previous maps never added.
    pub fn carry(&mut self, previous: &Globals) {
        for (name, value) in &previous.table {
            self.table.insert(name.clone(), *value);
        }
    }
    /// CEnvGlobal inputs. Returns Some(counter) for GetCounter (the Counter output) and
    /// None otherwise; false when the input is not an env_global input.
    pub fn input(&mut self, e: &Entity, input: &str, parameter: &str) -> (bool, Option<i32>) {
        let Some(name) = Self::name(e) else {
            return (false, None);
        };
        let value = parameter.trim().parse::<i32>().unwrap_or(0);
        let set = |table: &mut BTreeMap<String, (GlobalState, i32)>, state| {
            table.entry(name.clone()).or_insert((state, 0)).0 = state;
        };
        match input {
            "turnon" => set(&mut self.table, GlobalState::On),
            "turnoff" => set(&mut self.table, GlobalState::Off),
            "remove" => set(&mut self.table, GlobalState::Dead),
            "toggle" => match self.state(&name) {
                GlobalState::On => set(&mut self.table, GlobalState::Off),
                GlobalState::Off => set(&mut self.table, GlobalState::On),
                GlobalState::Dead => {}
            },
            "setcounter" | "addtocounter" | "getcounter" => {
                let entry = self.table.entry(name).or_insert((GlobalState::On, 0));
                match input {
                    "setcounter" => entry.1 = value,
                    "addtocounter" => entry.1 = entry.1.wrapping_add(value),
                    _ => return (true, Some(entry.1)),
                }
            }
            _ => return (false, None),
        }
        (true, None)
    }
}
fn int(e: &Entity, key: &str) -> i32 {
    e.get(key)
        .and_then(|v| v.trim().parse::<f32>().ok())
        .map_or(0, |v| v as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn global(name: &str, flags: &str, initial: &str) -> Entity {
        Entity {
            properties: [
                ("classname", "env_global"),
                ("globalstate", name),
                ("spawnflags", flags),
                ("initialstate", initial),
            ]
            .into_iter()
            .map(|(k, v)| (k.into(), v.into()))
            .collect(),
        }
    }
    fn world(entities: Vec<Entity>) -> World {
        World {
            entities,
            ..Default::default()
        }
    }

    #[test]
    fn trainstation_invulnerability_follows_env_global() {
        // d1_trainstation_03: set on at spawn, a trigger turns it off.
        let station = world(vec![global("gordon_invulnerable", "1", "1")]);
        let mut globals = Globals::spawn(&station);
        assert!(globals.is_on("GORDON_INVULNERABLE"));
        assert_eq!(
            globals.input(&station.entities[0], "turnoff", ""),
            (true, None)
        );
        assert!(!globals.is_on("gordon_invulnerable"));
        // d1_trainstation_04's env_global (spawnflags 0) adds nothing; the carried
        // off state stays; a later map with SF_GLOBAL_SET does not override it.
        let next = world(vec![global("gordon_invulnerable", "0", "0")]);
        let mut carried = Globals::spawn(&next);
        assert_eq!(carried.state("gordon_invulnerable"), GlobalState::Off);
        let mut later = Globals::spawn(&station);
        later.carry(&globals);
        assert!(!later.is_on("gordon_invulnerable"));
        carried.carry(&globals);
        assert!(carried.input(&next.entities[0], "toggle", "").0);
        assert!(carried.is_on("gordon_invulnerable"));
        assert!(carried.input(&next.entities[0], "remove", "").0);
        assert!(carried.input(&next.entities[0], "toggle", "").0);
        assert_eq!(carried.state("gordon_invulnerable"), GlobalState::Dead);
    }

    #[test]
    fn counters_add_the_global_turned_on() {
        let map = world(vec![global("counted", "0", "0")]);
        let mut globals = Globals::spawn(&map);
        assert_eq!(globals.state("counted"), GlobalState::Off);
        globals.input(&map.entities[0], "addtocounter", "3");
        assert_eq!(globals.state("counted"), GlobalState::On);
        assert_eq!(
            globals.input(&map.entities[0], "getcounter", ""),
            (true, Some(3))
        );
        assert_eq!(globals.input(&map.entities[0], "use", ""), (false, None));
    }
}
