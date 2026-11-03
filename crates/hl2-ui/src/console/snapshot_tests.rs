use super::Console;
use source_ui::console::panel::Level;

#[test]
fn live_viewmodel_tuning_log_replaces_its_snapshot_and_keeps_other_output() {
    let mut console = Console::default();
    console.shell.panel.clear_output();
    console.log("before snapshot");
    console.log_snapshot("{\"fov_offset\": 0.1}");
    console.log("unrelated output");
    console.log_snapshot("{\"fov_offset\": 0.2}");

    assert_eq!(console.shell.panel.output.len(), 3);
    assert_eq!(console.shell.panel.output[0].text, "before snapshot");
    assert_eq!(console.shell.panel.output[2].level, Level::Snapshot);
    assert_eq!(console.shell.panel.output[2].text, "{\"fov_offset\": 0.2}");
    assert_eq!(console.shell.panel.output[1].text, "unrelated output");

    console.shell.panel.clear_output();
    console.log_snapshot("{\"fov_offset\": 0.3}");
    assert_eq!(console.shell.panel.output.len(), 1);
    assert_eq!(console.shell.panel.output[0].text, "{\"fov_offset\": 0.3}");
}
