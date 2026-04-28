use bevy::prelude::*;

use crate::flag::{CarryingFlag, Flag, FlagState};
use crate::movement::Velocity;
use crate::player::{Facing, Ship, Thrusting, SHIP_RADIUS};
use crate::GameSet;

pub const TAG_RANGE: f32 = SHIP_RADIUS * 2.6;
pub const TAG_CONE_HALF_ANGLE: f32 = std::f32::consts::FRAC_PI_4; // 45° → 90° cone
pub const RESPAWN_SECS: f32 = 2.0;

#[derive(Component)]
pub struct Tagable;

#[derive(Component)]
pub struct RespawnPoint(pub Vec2);

#[derive(Component)]
pub struct Respawning {
    pub timer: Timer,
    pub point: Vec2,
}

pub struct TagPlugin;
impl Plugin for TagPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, (detect_tag, tick_respawn).in_set(GameSet::Gameplay));
    }
}

fn detect_tag(
    mut commands: Commands,
    ships: Query<
        (Entity, &Transform, &Ship, &Facing, Option<&CarryingFlag>, &Thrusting),
        (With<Tagable>, Without<Respawning>),
    >,
    mut flags: Query<&mut Flag>,
    respawn_points: Query<&RespawnPoint>,
    mut visibilities: Query<&mut Visibility>,
) {
    let entries: Vec<_> = ships
        .iter()
        .map(|(e, tf, s, f, c, t)| {
            (e, tf.translation.truncate(), s.team, f.0, c.map(|cc| cc.0), t.0)
        })
        .collect();

    let mut tagged: std::collections::HashSet<Entity> = std::collections::HashSet::new();

    for i in 0..entries.len() {
        let (_a_entity, a_pos, a_team, a_facing, a_carrying, a_thrust) = entries[i];
        let forward = Vec2::new(a_facing.cos(), a_facing.sin());
        for j in 0..entries.len() {
            if i == j { continue; }
            let (b_entity, b_pos, b_team, _, b_carrying, _) = entries[j];
            if a_team == b_team { continue; }
            if tagged.contains(&b_entity) { continue; }

            let diff = b_pos - a_pos;
            let dist = diff.length();
            if dist > TAG_RANGE || dist < 0.0001 { continue; }

            // Use the authoritative flag state, not the entries snapshot —
            // pickup's CarryingFlag insert may still be deferred.
            let _ = b_carrying;
            let b_carries = flags
                .iter()
                .any(|f| matches!(f.state, FlagState::Carried(c) if c == b_entity));

            // Carriers are normally prey, not predators — boosting just runs
            // them home faster, doesn't turn them into a battering ram.
            // EXCEPTION: when both ships carry, they mutually annihilate
            // (the loop runs again with roles swapped, so each gets tagged).
            if a_carrying.is_some() && !b_carries {
                continue;
            }

            if b_carries {
                // Carriers die on any contact — no facing-cone requirement,
                // no boost requirement. Bumping their side counts.
            } else {
                // Non-carrier targets are only vulnerable to a sprinting
                // attacker hitting them in their forward cone.
                if !a_thrust {
                    continue;
                }
                let dir = diff / dist;
                let dot = forward.dot(dir).clamp(-1.0, 1.0);
                if dot.acos() > TAG_CONE_HALF_ANGLE {
                    continue;
                }
            }

            // Drop any flag currently marked as carried by b.
            for mut flag in flags.iter_mut() {
                if matches!(flag.state, FlagState::Carried(c) if c == b_entity) {
                    flag.state = FlagState::Dropped(b_pos);
                }
            }
            commands.entity(b_entity).remove::<CarryingFlag>();
            if let Ok(rp) = respawn_points.get(b_entity) {
                commands.entity(b_entity).insert(Respawning {
                    timer: Timer::from_seconds(RESPAWN_SECS, TimerMode::Once),
                    point: rp.0,
                });
                if let Ok(mut vis) = visibilities.get_mut(b_entity) {
                    *vis = Visibility::Hidden;
                }
                tagged.insert(b_entity);
            }
        }
    }
}

fn tick_respawn(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(
        Entity,
        &mut Respawning,
        &mut Transform,
        &mut Visibility,
        &mut Velocity,
    )>,
) {
    for (entity, mut respawn, mut tf, mut vis, mut vel) in &mut q {
        respawn.timer.tick(time.delta());
        if respawn.timer.finished() {
            tf.translation.x = respawn.point.x;
            tf.translation.y = respawn.point.y;
            vel.0 = Vec2::ZERO;
            *vis = Visibility::Visible;
            commands.entity(entity).remove::<Respawning>();
        }
    }
}
