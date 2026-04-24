use bevy::prelude::*;
use bevy::sprite::{MaterialMesh2dBundle, Mesh2dHandle};

use crate::flag::CarryingFlag;
use crate::player::{Facing, Ship, Thrusting};
use crate::tag::Respawning;
use crate::GameSet;

#[derive(Component)]
pub struct TrailParticle {
    pub life: f32,
    pub max_life: f32,
}

pub struct TrailPlugin;
impl Plugin for TrailPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                spawn_carrier_trail,
                spawn_thrust_flame,
                pulse_thrust_ship,
                fade_trail,
            )
                .in_set(GameSet::Hud),
        );
    }
}

fn spawn_carrier_trail(
    mut commands: Commands,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    carriers: Query<(&Transform, &Ship), (With<CarryingFlag>, Without<Respawning>)>,
    mut acc: Local<f32>,
) {
    *acc += time.delta_seconds();
    if *acc < 0.05 {
        return;
    }
    *acc = 0.0;

    for (tf, ship) in &carriers {
        commands.spawn((
            MaterialMesh2dBundle {
                mesh: Mesh2dHandle(meshes.add(Circle::new(5.0))),
                material: materials.add(ship.team.color()),
                transform: Transform::from_translation(tf.translation.truncate().extend(-0.5)),
                ..default()
            },
            TrailParticle { life: 1.5, max_life: 1.5 },
            crate::game::PlayingEntity,
        ));
    }
}

fn spawn_thrust_flame(
    mut commands: Commands,
    time: Res<Time>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    ships: Query<(&Transform, &Thrusting, &Facing), Without<Respawning>>,
    mut acc: Local<f32>,
) {
    *acc += time.delta_seconds();
    if *acc < 0.03 {
        return;
    }
    *acc = 0.0;

    for (tf, thrust, facing) in &ships {
        if !thrust.0 {
            continue;
        }
        let forward = Vec2::new(facing.0.cos(), facing.0.sin());
        let behind = tf.translation.truncate() - forward * 20.0;
        commands.spawn((
            MaterialMesh2dBundle {
                mesh: Mesh2dHandle(meshes.add(Circle::new(7.0))),
                material: materials.add(Color::srgba(1.0, 0.7, 0.2, 0.9)),
                transform: Transform::from_translation(behind.extend(-0.5)),
                ..default()
            },
            TrailParticle { life: 0.35, max_life: 0.35 },
            crate::game::PlayingEntity,
        ));
    }
}

/// Brighten the ship itself while thrusting so you can tell at a glance.
fn pulse_thrust_ship(
    ships: Query<(&Ship, &Thrusting, &Handle<ColorMaterial>)>,
    mut materials: ResMut<Assets<ColorMaterial>>,
) {
    for (ship, thrust, handle) in &ships {
        let Some(mat) = materials.get_mut(handle) else { continue };
        let base = ship.team.color().to_linear();
        mat.color = if thrust.0 {
            Color::linear_rgba(
                (base.red * 1.6 + 0.2).min(1.0),
                (base.green * 1.6 + 0.2).min(1.0),
                (base.blue * 1.6 + 0.2).min(1.0),
                1.0,
            )
        } else {
            Color::linear_rgba(base.red, base.green, base.blue, 1.0)
        };
    }
}

fn fade_trail(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut TrailParticle, &mut Transform)>,
) {
    for (e, mut p, mut tf) in &mut q {
        p.life -= time.delta_seconds();
        if p.life <= 0.0 {
            commands.entity(e).despawn();
            continue;
        }
        let t = p.life / p.max_life;
        tf.scale = Vec3::splat(t.max(0.05));
    }
}
