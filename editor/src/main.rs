//! Editor with your game connected to it as a plugin.
use fyrox::event_loop::EventLoop;
use fyroxed_base::{Editor, StartupData};
use station_iapetus::{
    door::{DoorDirection, DoorState},
    GameConstructor,
};

fn main() {
    let event_loop = EventLoop::new();
    let mut editor = Editor::new(
        &event_loop,
        Some(StartupData {
            working_directory: Default::default(),
            scene: "data/levels/loading_bay.rgs".into(),
        }),
    );

    let editors = &editor.inspector.property_editors;
    editors.register_inheritable_enum::<DoorState, _>();
    editors.register_inheritable_enum::<DoorDirection, _>();

    editor.add_game_plugin(GameConstructor);
    editor.run(event_loop)
}
