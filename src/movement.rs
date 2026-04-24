use bevy::prelude::*;

use crate::player::Facing;
use crate::tag::Respawning;
use crate::GameSet;

#[derive(Component)]
pub struct Velocity(pub Vec2);

#[derive(Component)]
pub struct MaxSpeed(pub f32);

pub struct MovementPlugin;
impl Plugin for MovementPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (apply_velocity, face_velocity, apply_facing)
                .chain()
                .in_set(GameSet::Movement),
        );
    }
}

fn apply_velocity(
    mut q: Query<(&mut Transform, &Velocity), Without<Respawning>>,
    time: Res<Time>,
) {
    let dt = time.delta_seconds();
    for (mut tf, vel) in &mut q {
        tf.translation.x += vel.0.x * dt;
        tf.translation.y += vel.0.y * dt;
    }
}

fn face_velocity(mut q: Query<(&mut Facing, &Velocity), Without<Respawning>>) {
    for (mut facing, vel) in &mut q {
        // Only rotate when meaningfully moving — prevents jitter at rest.
        if vel.0.length_squared() > 400.0 {
            facing.0 = vel.0.y.atan2(vel.0.x);
        }
    }
}

fn apply_facing(mut q: Query<(&mut Transform, &Facing)>) {
    for (mut tf, facing) in &mut q {
        tf.rotation = Quat::from_rotation_z(facing.0);
    }
}
