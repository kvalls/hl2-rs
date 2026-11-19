//! The entity I/O log Source prints at `developer 2` (Source SDK game/server: cbase.cpp's
//! CBaseEntityOutput::FireOutput for every output fired, baseentity.cpp's AcceptInput for every
//! input an entity takes), for comparisons with native console logs. The host turns it on; while
//! it is off nothing is recorded.

use super::Output;
use modkit_core::World;

/// One output as it fired.
struct Fired {
    time: f64,
    caller: usize,
    target: String,
    input: String,
    delay: f64,
    parameter: String,
}

/// One input as an entity took it.
struct Taken {
    time: f64,
    caller: usize,
    target: usize,
    input: String,
    parameter: String,
}

enum Traced {
    Output(Fired),
    Input(Taken),
}

/// The outputs fired and inputs taken since the host last took them.
#[derive(Default)]
pub struct IoTrace {
    traced: Vec<Traced>,
}

impl IoTrace {
    pub(super) fn record(&mut self, time: f64, caller: usize, output: &Output) {
        self.traced.push(Traced::Output(Fired {
            time,
            caller,
            target: output.target.clone(),
            input: output.input.clone(),
            delay: output.delay,
            parameter: output.parameter.clone(),
        }));
    }

    pub(super) fn record_input(
        &mut self,
        time: f64,
        (caller, target): (usize, usize),
        input: &str,
        parameter: &str,
    ) {
        self.traced.push(Traced::Input(Taken {
            time,
            caller,
            target,
            input: input.into(),
            parameter: parameter.into(),
        }));
    }

    /// The lines since the last call as Source prints them: `(time) output: (class,name) ->
    /// (target,input)(parameter)` with the delay after the input when there is one, and
    /// `(time) input caller: target.Input(parameter)` (the target by its name, else its class).
    pub fn take_lines(&mut self, world: &World) -> Vec<String> {
        std::mem::take(&mut self.traced)
            .into_iter()
            .map(|t| match t {
                Traced::Output(f) => line(&f, world),
                Traced::Input(i) => input_line(&i, world),
            })
            .collect()
    }
}

fn input_line(i: &Taken, world: &World) -> String {
    let name = |id: usize| world.entities.get(id).and_then(|e| e.get("targetname"));
    let caller = name(i.caller).unwrap_or("");
    let target = match world.entities.get(i.target) {
        Some(e) => name(i.target)
            .filter(|n| !n.is_empty())
            .unwrap_or(e.class()),
        None => "player",
    };
    format!(
        "({:.2}) input {caller}: {target}.{}({})",
        i.time, i.input, i.parameter
    )
}

fn line(f: &Fired, world: &World) -> String {
    let (class, name) = world.entities.get(f.caller).map_or(("NULL", "NULL"), |e| {
        (e.class(), e.get("targetname").unwrap_or(""))
    });
    let delay = if f.delay != 0. {
        format!(",{:.1}", f.delay)
    } else {
        String::new()
    };
    format!(
        "({:.2}) output: ({class},{name}) -> ({},{}{delay})({})",
        f.time, f.target, f.input, f.parameter
    )
}

#[cfg(test)]
mod tests {
    use super::super::Scene;
    use super::*;
    use modkit_core::Entity;

    fn relay(name: &str, outputs: &[(&str, &str)]) -> Entity {
        let named = [("classname", "logic_relay"), ("targetname", name)];
        Entity {
            properties: named
                .into_iter()
                .chain(outputs.iter().copied())
                .map(|(k, v)| (k.into(), v.into()))
                .collect(),
        }
    }

    #[test]
    fn fired_outputs_read_like_the_native_developer_log() {
        let world = World {
            entities: vec![
                relay(
                    "start",
                    &[
                        ("OnTrigger", "train,SetSpeed,0.6,0.6,1"),
                        ("OnTrigger", "relay2,Trigger,,0,-1"),
                    ],
                ),
                relay("relay2", &[]),
            ],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        assert!(scene.io_trace.is_none(), "off unless the host turns it on");
        scene.io_trace = Some(IoTrace::default());
        scene.time = 62.374;
        scene.fire(0, "OnTrigger", 0);
        let mut lines = scene.io_trace.as_mut().unwrap().take_lines(&world);
        // The input lines come as the events are delivered: relay2 exists, train does not.
        scene.tick(&world, glam::Vec3::ZERO, 0.015);
        let inputs = scene.io_trace.as_mut().unwrap().take_lines(&world);
        assert_eq!(inputs, ["(62.39) input start: relay2.Trigger()"]);
        lines.retain(|l| l.contains("output"));
        assert_eq!(
            lines,
            [
                "(62.37) output: (logic_relay,start) -> (train,SetSpeed,0.6)(0.6)",
                "(62.37) output: (logic_relay,start) -> (relay2,Trigger)()",
            ]
        );
        assert!(scene
            .io_trace
            .as_mut()
            .unwrap()
            .take_lines(&world)
            .is_empty());
    }
}
