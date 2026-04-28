use bevy::prelude::*;

pub mod arena;
pub mod bot;
pub mod flag;
pub mod game;
pub mod hud;
pub mod input;
pub mod lobby;
pub mod movement;
pub mod net;
pub mod net_smoke_test;
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
        // Sim sets only run on whoever owns the authoritative simulation
        // (Solo or OnlineHost). On OnlineClient these stay dormant and
        // `client_apply_snapshot` writes the world from the host's stream.
        // Input + Hud always run so the client can capture local input
        // (forwarded to host in S3 task 10) and render the UI.
        .configure_sets(
            Update,
            (GameSet::Ai, GameSet::Movement, GameSet::Physics, GameSet::Gameplay)
                .run_if(net::is_local_authority),
        )
        .add_plugins((
            game::GamePlugin,
            arena::ArenaPlugin,
            player::PlayerPlugin,
            input::InputPlugin,
            lobby::LobbyPlugin,
            movement::MovementPlugin,
            net::NetPlugin,
            // net_smoke_test::NetSmokeTestPlugin — STANDBY ONLY.
            // We confirmed at SDK 35 that bindProcessToNetwork now
            // succeeds and DNS works, so the NDK per-socket-binding
            // path in `android_net_bind` isn't needed for shipping.
            // Keeping the plugin + crate around so if a future Android
            // version (16+) breaks process-wide binding again, we have
            // a tested path to fall back on. See android_net_bind/README
            // and src/net_smoke_test.rs.
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
