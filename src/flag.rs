use bevy::prelude::*;
use bevy::sprite::{MaterialMesh2dBundle, Mesh2dHandle};

use crate::arena::{Base, BASE_RADIUS};
use crate::player::{Ship, SHIP_RADIUS};
use crate::tag::Respawning;
use crate::team::Team;
use crate::GameSet;

pub const FLAG_RADIUS: f32 = 12.0;

#[derive(Clone, Copy, PartialEq)]
pub enum FlagState {
    Home,
    Carried(Entity),
    Dropped(Vec2),
}

#[derive(Component)]
pub struct Flag {
    pub team: Team,
    pub state: FlagState,
    pub home: Vec2,
}

#[derive(Component)]
pub struct CarryingFlag(pub Entity);

#[derive(Resource, Default)]
pub struct Score {
    pub red: u32,
    pub blue: u32,
}

#[derive(Event)]
pub struct CaptureEvent {
    pub team: Team,
}

pub struct FlagPlugin;
impl Plugin for FlagPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Score>()
            .add_event::<CaptureEvent>()
            .add_systems(
                Update,
                (pickup_or_return, carry_flag, capture_flag, pulse_dropped)
                    .chain()
                    .in_set(GameSet::Gameplay),
            );
    }
}

pub fn spawn_flags(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    bases: Query<(&Transform, &Base)>,
) {
    for (tf, base) in &bases {
        let pos = tf.translation.truncate();
        commands.spawn((
            MaterialMesh2dBundle {
                mesh: Mesh2dHandle(meshes.add(Rectangle::new(FLAG_RADIUS * 2.0, FLAG_RADIUS * 2.0))),
                material: materials.add(base.team.color()),
                transform: Transform::from_translation(pos.extend(2.0)),
                ..default()
            },
            Flag { team: base.team, state: FlagState::Home, home: pos },
            crate::game::PlayingEntity,
        ));
    }
}

/// Enemy touch picks it up; own-team touch on a *dropped* flag returns it home.
fn pickup_or_return(
    mut commands: Commands,
    mut flags: Query<(Entity, &mut Flag)>,
    ships: Query<(Entity, &Transform, &Ship, Option<&CarryingFlag>), Without<Respawning>>,
) {
    for (flag_entity, mut flag) in &mut flags {
        let flag_pos = match flag.state {
            FlagState::Home => flag.home,
            FlagState::Dropped(p) => p,
            FlagState::Carried(_) => continue,
        };

        for (ship_entity, ship_tf, ship, carrying) in &ships {
            let dist = flag_pos.distance(ship_tf.translation.truncate());
            if dist > SHIP_RADIUS + FLAG_RADIUS {
                continue;
            }

            if ship.team == flag.team {
                // Own team can touch a dropped flag to return it home —
                // allowed even while the ship is already carrying the enemy
                // flag (it's a touch, not a pickup).
                if matches!(flag.state, FlagState::Dropped(_)) {
                    flag.state = FlagState::Home;
                }
            } else if carrying.is_none() {
                // Enemy team grabs — but only if they aren't already carrying.
                flag.state = FlagState::Carried(ship_entity);
                commands.entity(ship_entity).insert(CarryingFlag(flag_entity));
                break;
            }
        }
    }
}

fn carry_flag(
    mut flags: Query<(&mut Transform, &Flag)>,
    carriers: Query<
        &Transform,
        (With<CarryingFlag>, Without<Flag>, Without<crate::tag::Respawning>),
    >,
) {
    for (mut flag_tf, flag) in &mut flags {
        match flag.state {
            FlagState::Carried(carrier) => {
                if let Ok(c_tf) = carriers.get(carrier) {
                    flag_tf.translation = c_tf.translation.truncate().extend(2.0)
                        + Vec3::new(0.0, 24.0, 0.0);
                }
            }
            FlagState::Home => {
                flag_tf.translation = flag.home.extend(2.0);
            }
            FlagState::Dropped(p) => {
                flag_tf.translation = p.extend(2.0);
            }
        }
    }
}

fn capture_flag(
    mut commands: Commands,
    mut flags: Query<&mut Flag>,
    bases: Query<(&Transform, &Base)>,
    carriers: Query<(Entity, &Transform, &Ship, &CarryingFlag)>,
    mut score: ResMut<Score>,
    mut ev: EventWriter<CaptureEvent>,
) {
    for (carrier_entity, carrier_tf, ship, carrying) in &carriers {
        // Own flag must not be in enemy hands. Home or dropped-in-field both count as safe.
        let own_safe = flags
            .iter()
            .any(|f| f.team == ship.team && !matches!(f.state, FlagState::Carried(_)));
        if !own_safe { continue; }

        // Must be touching own base
        let at_base = bases.iter().any(|(btf, b)| {
            b.team == ship.team
                && btf.translation.truncate().distance(carrier_tf.translation.truncate())
                    < BASE_RADIUS
        });
        if !at_base { continue; }

        // Guard: if the carried flag isn't actually Carried (e.g. we already
        // captured it this frame and the deferred CarryingFlag removal hasn't
        // applied yet), skip rather than double-counting.
        let Ok(mut flag) = flags.get_mut(carrying.0) else { continue };
        if !matches!(flag.state, FlagState::Carried(_)) { continue; }

        flag.state = FlagState::Home;
        commands.entity(carrier_entity).remove::<CarryingFlag>();
        match ship.team {
            Team::Red => score.red += 1,
            Team::Blue => score.blue += 1,
        }
        ev.send(CaptureEvent { team: ship.team });
    }
}

/// Gentle pulse on dropped flags so they're visually distinct.
fn pulse_dropped(time: Res<Time>, mut q: Query<(&Flag, &mut Transform)>) {
    let t = time.elapsed_seconds();
    for (flag, mut tf) in &mut q {
        let scale = if matches!(flag.state, FlagState::Dropped(_)) {
            1.0 + (t * 6.0).sin() * 0.2
        } else {
            1.0
        };
        tf.scale = Vec3::splat(scale);
    }
}
