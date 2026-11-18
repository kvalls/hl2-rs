//! The entity I/O log Source prints at `developer 2` (CBaseEntityOutput::FireOutput's DevMsg,
//! Source SDK game/server/cbase.cpp): every output fired, when, by whom and to where. It exists
//! for comparisons with native console logs; the host turns it on, and while it is off firing an
//! output records nothing.

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

/// The outputs fired since the host last took them.
#[derive(Default)]
pub struct IoTrace {
    fired: Vec<Fired>,
}

impl IoTrace {
    pub(super) fn record(&mut self, time: f64, caller: usize, output: &Output) {
        self.fired.push(Fired {
            time,
            caller,
            target: output.target.clone(),
            input: output.input.clone(),
            delay: output.delay,
            parameter: output.parameter.clone(),
        });
    }

    /// The outputs since the last call as Source prints them: `(time) output: (class,name) ->
    /// (target,input)(parameter)`, with the delay after the input when there is one.
    pub fn take_lines(&mut self, world: &World) -> Vec<String> {
        std::mem::take(&mut self.fired)
            .into_iter()
            .map(|f| line(&f, world))
            .collect()
    }
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
            entities: vec![relay(
                "start",
                &[
                    ("OnTrigger", "train,SetSpeed,0.6,0.6,1"),
                    ("OnTrigger", "relay2,Trigger,,0,-1"),
                ],
            )],
            ..Default::default()
        };
        let mut scene = Scene::new(&world);
        assert!(scene.io_trace.is_none(), "off unless the host turns it on");
        scene.io_trace = Some(IoTrace::default());
        scene.time = 62.374;
        scene.fire(0, "OnTrigger", 0);
        let lines = scene.io_trace.as_mut().unwrap().take_lines(&world);
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
