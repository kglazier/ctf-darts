use bevy::prelude::*;

pub mod arena;
pub mod bot;
pub mod flag;
pub mod game;
pub mod hud;
pub mod input;
pub mod movement;
pub mod physics;
pub mod player;
pub mod projectile;
pub mod tag;
pub mod team;
pub mod trail;

pub const ARENA_WIDTH: f32 = 1600.0;
pub const ARENA_HEIGHT: f32 = 900.0;

#[derive(SystemSet, Clone, Debug, PartialEq, Eq, Hash)]
pub enum GameSet {
    Input,
    Ai,
    Movement,
    Physics,
    Gameplay,
    Hud,
}

#[bevy_main]
pub fn main() {
    App::new()
        .add_plugins(
            DefaultPlugins
                .set(WindowPlugin {
                    primary_window: Some(Window {
                        title: "Space Boosters".into(),
                        resizable: true,
                        ..default()
                    }),
                    ..default()
                }),
        )
        .insert_resource(ClearColor(Color::srgb(0.02, 0.02, 0.06)))
        .configure_sets(
            Update,
            (
                GameSet::Input,
                GameSet::Ai,
                GameSet::Movement,
                GameSet::Physics,
                GameSet::Gameplay,
                GameSet::Hud,
            )
                .chain()
                .run_if(game::playing_unpaused),
        )
        .add_plugins((
            game::GamePlugin,
            arena::ArenaPlugin,
            player::PlayerPlugin,
            input::InputPlugin,
            movement::MovementPlugin,
            physics::PhysicsPlugin,
            flag::FlagPlugin,
            tag::TagPlugin,
            bot::BotPlugin,
            projectile::ProjectilePlugin,
            trail::TrailPlugin,
            hud::HudPlugin,
        ))
        .run();
}
