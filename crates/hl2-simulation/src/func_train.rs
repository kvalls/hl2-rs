//! func_train along path_corners (SDK trains.cpp CFuncTrain). Activate puts the train's
//! bounds center on its first corner (origin = corner - OBBCenter); an unnamed train starts
//! 0.1 s later, a named one waits for Start (SF_TRAIN_WAIT_RETRIGGER). Next moves it in a line
//! to the next corner at its speed (a corner's own nonzero `speed` replaces it on arrival);
//! Wait passes the corner (InPass, which fires OnPass), stops at a corner flagged
//! SF_CORNER_WAIT_FOR_TRIG (1), waits the corner's `wait` seconds, or goes on at once. Stop
//! halts it, Toggle switches. SF_CORNER_TELEPORT, blocking damage and the move sounds are not
//! modeled.
use crate::entities::number;
use glam::Vec3;
use modkit_core::{Entity, World};
use std::collections::BTreeMap;

/// SF_TRAIN_WAIT_RETRIGGER / SF_CORNER_WAIT_FOR_TRIG.
const WAIT_FOR_TRIGGER: u32 = 1;

#[derive(Clone, Debug)]
struct Train {
    center: Vec3,
    speed: f32,
    /// The corner reached last (m_hCurrentTarget) and the name of the next (m_target).
    corner: Option<usize>,
    next: String,
    origin: Vec3,
    /// The move in progress: from, to, start time and duration.
    moving: Option<(Vec3, Vec3, f64, f64)>,
    wait_until: Option<f64>,
    stopped: bool,
}

#[derive(Clone, Debug, Default)]
pub struct FuncTrains {
    trains: BTreeMap<usize, Train>,
}

fn named(world: &World, name: &str) -> Option<usize> {
    (!name.is_empty()).then_some(())?;
    world.entities.iter().position(|e| {
        e.get("targetname")
            .is_some_and(|t| t.eq_ignore_ascii_case(name))
    })
}

fn target(e: &Entity) -> String {
    e.get("target").unwrap_or("").to_owned()
}

impl FuncTrains {
    /// CFuncTrain::Spawn and Activate of every func_train; returns each train's first origin.
    pub fn spawn(&mut self, world: &World, time: f64) -> Vec<(usize, Vec3)> {
        self.trains.clear();
        let mut placed = Vec::new();
        for (id, e) in world.entities.iter().enumerate() {
            if e.class() != "func_train" {
                continue;
            }
            let bounds = e
                .get("model")
                .and_then(|m| m.strip_prefix('*'))
                .and_then(|m| m.parse::<usize>().ok())
                .and_then(|m| world.brush_models.iter().find(|b| b.id == m));
            let center = bounds.map_or(Vec3::ZERO, |b| (b.mins + b.maxs) * 0.5);
            let speed = Some(number(e, "speed", 0.))
                .filter(|&s| s != 0.)
                .unwrap_or(100.);
            let corner = named(world, e.get("target").unwrap_or(""));
            let origin = corner.map_or(e.origin(), |c| world.entities[c].origin() - center);
            let named_train = e.get("targetname").is_some_and(|n| !n.is_empty());
            self.trains.insert(
                id,
                Train {
                    center,
                    speed,
                    corner,
                    next: corner
                        .map(|c| target(&world.entities[c]))
                        .unwrap_or_default(),
                    origin,
                    moving: None,
                    wait_until: (!named_train && corner.is_some()).then_some(time + 0.1),
                    stopped: named_train,
                },
            );
            placed.push((id, origin));
        }
        placed
    }

    /// Start, Stop and Toggle.
    pub fn input(&mut self, world: &World, id: usize, input: &str, time: f64) -> bool {
        let Some(train) = self.trains.get_mut(&id) else {
            return false;
        };
        let start = match input {
            "start" => true,
            "stop" => false,
            "toggle" => train.stopped,
            _ => return false,
        };
        if start && train.stopped {
            train.stopped = false;
            next(train, world, time);
        } else if !start && !train.stopped {
            train.stopped = true;
            train.moving = None;
            train.wait_until = None;
        }
        true
    }

    /// One tick: moving trains give `pose` their origin and `passed` each corner reached
    /// (for its OnPass).
    pub fn advance(
        &mut self,
        world: &World,
        time: f64,
        mut pose: impl FnMut(usize, Vec3),
        passed: &mut Vec<usize>,
    ) {
        for (&id, train) in &mut self.trains {
            if train.wait_until.is_some_and(|t| t <= time) && !train.stopped {
                train.wait_until = None;
                next(train, world, time);
            }
            let Some((from, to, start, duration)) = train.moving else {
                continue;
            };
            let share = ((time - start) / duration.max(1e-6)).clamp(0., 1.) as f32;
            train.origin = from.lerp(to, share);
            pose(id, train.origin);
            if share >= 1. {
                train.moving = None;
                if let Some(corner) = train.corner {
                    passed.push(corner);
                    arrive(train, &world.entities[corner], world, start + duration);
                }
            }
        }
    }
}

/// CFuncTrain::Wait at the corner just reached.
fn arrive(train: &mut Train, corner: &Entity, world: &World, time: f64) {
    if number(corner, "spawnflags", 0.) as u32 & WAIT_FOR_TRIGGER != 0 {
        train.stopped = true;
        return;
    }
    let wait = number(corner, "wait", 0.);
    if wait != 0. {
        train.wait_until = (wait > 0.).then_some(time + f64::from(wait));
    } else {
        next(train, world, time);
    }
}

/// CFuncTrain::Next: toward the next corner at the speed set by the corner left.
fn next(train: &mut Train, world: &World, time: f64) {
    let Some(corner) = named(world, &train.next) else {
        return;
    };
    if let Some(current) = train.corner {
        let speed = number(&world.entities[current], "speed", 0.);
        if speed != 0. {
            train.speed = speed;
        }
    }
    let e = &world.entities[corner];
    train.next = target(e);
    train.corner = Some(corner);
    let to = e.origin() - train.center;
    let duration = f64::from(to.distance(train.origin) / train.speed.max(1e-3));
    train.moving = Some((train.origin, to, time, duration));
}

#[cfg(test)]
mod tests {
    use super::*;
    use modkit_core::BrushModel;

    fn entity(pairs: &[(&str, &str)]) -> Entity {
        Entity {
            properties: pairs
                .iter()
                .map(|(k, v)| ((*k).into(), (*v).into()))
                .collect(),
        }
    }

    #[test]
    fn a_named_train_waits_for_start_then_loops_its_corners_at_its_speed() {
        let world = World {
            entities: vec![
                entity(&[
                    ("classname", "func_train"),
                    ("targetname", "zap"),
                    ("model", "*1"),
                    ("target", "a"),
                    ("speed", "80"),
                    ("origin", "0 0 0"),
                ]),
                entity(&[
                    ("classname", "path_corner"),
                    ("targetname", "a"),
                    ("target", "b"),
                    ("origin", "0 0 0"),
                ]),
                entity(&[
                    ("classname", "path_corner"),
                    ("targetname", "b"),
                    ("target", "a"),
                    ("origin", "160 0 0"),
                ]),
            ],
            brush_models: vec![BrushModel {
                id: 1,
                surfaces: Vec::new(),
                brushes: Vec::new(),
                mins: Vec3::new(-8., -8., -8.),
                maxs: Vec3::new(8., 8., 24.),
            }],
            ..Default::default()
        };
        let mut trains = FuncTrains::default();
        let placed = trains.spawn(&world, 0.);
        // The bounds center (0 0 8) sits on corner a.
        assert_eq!(placed, vec![(0, Vec3::new(0., 0., -8.))]);
        let mut passed = Vec::new();
        let step = |trains: &mut FuncTrains, time: f64, passed: &mut Vec<usize>| {
            let mut poses = Vec::new();
            trains.advance(&world, time, |id, origin| poses.push((id, origin)), passed);
            poses
        };
        assert!(step(&mut trains, 1., &mut passed).is_empty());
        assert!(trains.input(&world, 0, "start", 1.));
        assert_eq!(
            step(&mut trains, 2., &mut passed),
            vec![(0, Vec3::new(80., 0., -8.))]
        );
        step(&mut trains, 3., &mut passed);
        assert_eq!(passed, vec![2]);
        // Back toward a.
        assert_eq!(
            step(&mut trains, 4., &mut passed),
            vec![(0, Vec3::new(80., 0., -8.))]
        );
        trains.input(&world, 0, "stop", 4.);
        assert!(step(&mut trains, 5., &mut passed).is_empty());
    }
}
