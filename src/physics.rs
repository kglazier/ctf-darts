use bevy::prelude::*;

use crate::arena::{Base, Wall, BASE_RADIUS};
use crate::flag::{CarryingFlag, Flag, FlagState};
use crate::movement::Velocity;
use crate::player::Ship;
use crate::tag::Respawning;
use crate::team::Team;
use crate::GameSet;

#[derive(Component, Clone)]
pub enum Collider {
    Circle { radius: f32 },
}

pub struct PhysicsPlugin;
impl Plugin for PhysicsPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(
            Update,
            (
                resolve_ship_collisions,
                push_own_team_from_flag,
                resolve_wall_collisions,
            )
                .chain()
                .in_set(GameSet::Physics),
        );
    }
}

/// Ship-on-ship collisions: equal-mass elastic exchange along the contact normal,
/// lightly damped. Gives a TagPro-style "you can push people around" feel without
/// needing a physics engine.
fn resolve_ship_collisions(
    mut q: Query<(&mut Transform, &mut Velocity, &Collider), (With<Ship>, Without<Respawning>)>,
) {
    let mut pairs = q.iter_combinations_mut();
    while let Some([(mut ta, mut va, ca), (mut tb, mut vb, cb)]) = pairs.fetch_next() {
        let ra = match ca {
            Collider::Circle { radius } => *radius,
        };
        let rb = match cb {
            Collider::Circle { radius } => *radius,
        };
        let pa = ta.translation.truncate();
        let pb = tb.translation.truncate();
        let diff = pb - pa;
        let dist = diff.length();
        let min_dist = ra + rb;
        if dist >= min_dist || dist < 0.01 {
            continue;
        }
        let normal = diff / dist;
        let overlap = (min_dist - dist) * 0.5;
        ta.translation.x -= normal.x * overlap;
        ta.translation.y -= normal.y * overlap;
        tb.translation.x += normal.x * overlap;
        tb.translation.y += normal.y * overlap;

        // Only exchange velocity if they're approaching along the normal.
        let va_n = va.0.dot(normal);
        let vb_n = vb.0.dot(normal);
        if va_n > vb_n {
            const BOUNCE: f32 = 0.9;
            let delta = (va_n - vb_n) * BOUNCE;
            va.0 -= normal * delta;
            vb.0 += normal * delta;
        }
    }
}

/// Anti-camp: a same-team non-carrier gets strongly pushed away from their
/// own flag when it's home — like a magnet, not a wall. A boosting ship can
/// still punch through briefly if they really want to. Carriers are exempt.
fn push_own_team_from_flag(
    time: Res<Time>,
    flags: Query<&Flag>,
    bases: Query<(&Transform, &Base), Without<Ship>>,
    mut ships: Query<
        (&Transform, &mut Velocity, &Ship, Option<&CarryingFlag>),
        (Without<Respawning>, Without<Base>),
    >,
) {
    use std::collections::HashSet;
    let home_teams: HashSet<Team> = flags
        .iter()
        .filter(|f| matches!(f.state, FlagState::Home))
        .map(|f| f.team)
        .collect();
    if home_teams.is_empty() {
        return;
    }

    let dt = time.delta_seconds();
    const MAGNET_ACCEL: f32 = 2400.0;

    for (tf, mut vel, ship, carrying) in &mut ships {
        if carrying.is_some() {
            continue;
        }
        if !home_teams.contains(&ship.team) {
            continue;
        }
        for (btf, base) in &bases {
            if base.team != ship.team {
                continue;
            }
            let bpos = btf.translation.truncate();
            let diff = tf.translation.truncate() - bpos;
            let d = diff.length();
            if d < BASE_RADIUS && d > 0.01 {
                let normal = diff / d;
                // Strongest push at the flag, easing off near the ring edge.
                let strength = 1.0 - d / BASE_RADIUS;
                vel.0 += normal * strength * MAGNET_ACCEL * dt;
            } else if d <= 0.01 {
                // Directly on top — give a full-strength shove outward.
                vel.0 += Vec2::X * MAGNET_ACCEL * dt;
            }
        }
    }
}

fn resolve_wall_collisions(
    mut ships: Query<(&mut Transform, &mut Velocity, &Collider)>,
    walls: Query<(&Transform, &Wall), Without<Velocity>>,
) {
    for (mut ship_tf, mut vel, col) in &mut ships {
        let radius = match col {
            Collider::Circle { radius } => *radius,
        };

        let mut pos = ship_tf.translation.truncate();
        let mut v = vel.0;

        for (wall_tf, wall) in &walls {
            let wall_center = wall_tf.translation.truncate();
            let diff = pos - wall_center;
            let clamped = Vec2::new(
                diff.x.clamp(-wall.half_extents.x, wall.half_extents.x),
                diff.y.clamp(-wall.half_extents.y, wall.half_extents.y),
            );
            let closest = wall_center + clamped;
            let offset = pos - closest;
            let dist = offset.length();
            if dist < radius && dist > 0.0001 {
                let normal = offset / dist;
                pos = closest + normal * radius;
                let dot = v.dot(normal);
                if dot < 0.0 {
                    v -= normal * dot;
                }
            } else if dist <= 0.0001 {
                let push = if wall.half_extents.x - diff.x.abs() < wall.half_extents.y - diff.y.abs() {
                    Vec2::new(diff.x.signum(), 0.0)
                } else {
                    Vec2::new(0.0, diff.y.signum())
                };
                pos = wall_center + push * (wall.half_extents * push.abs() + Vec2::splat(radius + 1.0));
            }
        }

        ship_tf.translation.x = pos.x;
        ship_tf.translation.y = pos.y;
        vel.0 = v;
    }
}
