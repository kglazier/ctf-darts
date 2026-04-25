use bevy::prelude::*;
use bevy::sprite::{MaterialMesh2dBundle, Mesh2dHandle};

use crate::arena::Wall;
use crate::bot::BotDifficulty;
use crate::flag::{CarryingFlag, Flag, FlagState};
use crate::input::PlayerInput;
use crate::movement::Velocity;
use crate::player::{Facing, PlayerControlled, Ship, SHIP_RADIUS};
use crate::tag::{RespawnPoint, Respawning, Tagable, RESPAWN_SECS};
use crate::team::Team;
use crate::GameSet;

pub const PROJECTILE_RADIUS: f32 = 6.0;
pub const PROJECTILE_SPEED: f32 = 640.0;
pub const PROJECTILE_LIFE_SECS: f32 = 1.6;
pub const FIRE_COOLDOWN_SECS: f32 = 0.4;

#[derive(Resource, Clone, Copy, PartialEq, Eq, Debug, Default)]
pub enum GameMode {
    #[default]
    Classic,
    Shooter,
}

impl GameMode {
    pub fn label(self) -> &'static str {
        match self {
            GameMode::Classic => "CLASSIC",
            GameMode::Shooter => "SHOOTER",
        }
    }
    pub fn next(self) -> Self {
        match self {
            GameMode::Classic => GameMode::Shooter,
            GameMode::Shooter => GameMode::Classic,
        }
    }
}

#[derive(Component)]
pub struct Projectile {
    pub team: Team,
    pub life: Timer,
}

#[derive(Component)]
pub struct FireCooldown(pub Timer);

impl Default for FireCooldown {
    fn default() -> Self {
        let mut t = Timer::from_seconds(FIRE_COOLDOWN_SECS, TimerMode::Once);
        t.set_elapsed(std::time::Duration::from_secs_f32(FIRE_COOLDOWN_SECS));
        Self(t)
    }
}

pub struct ProjectilePlugin;
impl Plugin for ProjectilePlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<GameMode>()
            .add_systems(Update, (player_shoot, bot_shoot).in_set(GameSet::Ai))
            .add_systems(
                Update,
                (
                    tick_projectile_life,
                    projectile_vs_projectile,
                    projectile_vs_walls,
                    projectile_vs_ships,
                )
                    .chain()
                    .in_set(GameSet::Gameplay),
            );
    }
}

fn spawn_projectile(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<ColorMaterial>>,
    origin: Vec2,
    direction: Vec2,
    team: Team,
) {
    let vel = direction.normalize_or_zero() * PROJECTILE_SPEED;
    commands.spawn((
        MaterialMesh2dBundle {
            mesh: Mesh2dHandle(meshes.add(Circle::new(PROJECTILE_RADIUS))),
            material: materials.add(team.color()),
            transform: Transform::from_translation(origin.extend(4.0)),
            ..default()
        },
        Projectile {
            team,
            life: Timer::from_seconds(PROJECTILE_LIFE_SECS, TimerMode::Once),
        },
        Velocity(vel),
        crate::game::PlayingEntity,
    ));
}

fn player_shoot(
    mode: Res<GameMode>,
    time: Res<Time>,
    input: Res<PlayerInput>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut players: Query<
        (&Transform, &Facing, &Ship, &mut FireCooldown),
        (With<PlayerControlled>, Without<Respawning>, Without<CarryingFlag>),
    >,
) {
    for (tf, facing, ship, mut cd) in &mut players {
        cd.0.tick(time.delta());
        if *mode != GameMode::Shooter {
            continue;
        }
        if !input.sprint {
            continue;
        }
        if !cd.0.finished() {
            continue;
        }
        let dir = Vec2::new(facing.0.cos(), facing.0.sin());
        let origin = tf.translation.truncate() + dir * (SHIP_RADIUS + PROJECTILE_RADIUS + 2.0);
        spawn_projectile(&mut commands, &mut meshes, &mut materials, origin, dir, ship.team);
        cd.0.reset();
    }
}

/// Bots fire when an enemy sits in their forward cone within difficulty-scaled range.
fn bot_shoot(
    mode: Res<GameMode>,
    time: Res<Time>,
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    mut shooters: Query<
        (
            &Transform,
            &Facing,
            &Ship,
            &BotDifficulty,
            &mut FireCooldown,
            Option<&Respawning>,
        ),
        (Without<PlayerControlled>, Without<CarryingFlag>),
    >,
    all_ships: Query<(&Transform, &Ship), Without<Respawning>>,
) {
    let now = time.elapsed_seconds();
    for (tf, facing, ship, difficulty, mut cd, respawning) in &mut shooters {
        cd.0.tick(time.delta());
        if *mode != GameMode::Shooter {
            continue;
        }
        if respawning.is_some() {
            continue;
        }
        if !cd.0.finished() {
            continue;
        }

        let pos = tf.translation.truncate();
        let forward = Vec2::new(facing.0.cos(), facing.0.sin());

        // Difficulty curves — Easy shoots slowly with big aim jitter; Hard is
        // the previous sniper.
        let (fire_range, cooldown_secs, jitter_rad): (f32, f32, f32) = match *difficulty {
            BotDifficulty::Easy => (180.0, 1.0, 0.28),
            BotDifficulty::Medium => (320.0, 0.55, 0.10),
            // Hard is one notch above Medium — slightly longer range and
            // faster cooldown, but still meaningfully off-perfect aim.
            BotDifficulty::Hard => (380.0, 0.45, 0.06),
        };
        const CONE_COS: f32 = 0.94;

        let mut should_fire = false;
        for (etf, eship) in &all_ships {
            if eship.team == ship.team {
                continue;
            }
            let rel = etf.translation.truncate() - pos;
            let dist = rel.length();
            if dist > fire_range || dist < 1.0 {
                continue;
            }
            let dir = rel / dist;
            if forward.dot(dir) > CONE_COS {
                should_fire = true;
                break;
            }
        }

        if should_fire {
            // Deterministic pseudo-random jitter — avoids adding a rand dep.
            let seed = (now * 19.37 + tf.translation.x * 0.13 + tf.translation.y * 0.17).sin();
            let aim_angle = facing.0 + seed * jitter_rad;
            let aim_dir = Vec2::new(aim_angle.cos(), aim_angle.sin());
            let origin = pos + forward * (SHIP_RADIUS + PROJECTILE_RADIUS + 2.0);
            spawn_projectile(&mut commands, &mut meshes, &mut materials, origin, aim_dir, ship.team);
            cd.0 = Timer::from_seconds(cooldown_secs, TimerMode::Once);
        }
    }
}

fn tick_projectile_life(
    mut commands: Commands,
    time: Res<Time>,
    mut q: Query<(Entity, &mut Projectile)>,
) {
    for (e, mut p) in &mut q {
        p.life.tick(time.delta());
        if p.life.finished() {
            commands.entity(e).despawn();
        }
    }
}

/// Opposing-team projectiles that overlap despawn each other.
fn projectile_vs_projectile(
    mut commands: Commands,
    projectiles: Query<(Entity, &Transform, &Projectile)>,
) {
    let entries: Vec<(Entity, Vec2, Team)> = projectiles
        .iter()
        .map(|(e, tf, p)| (e, tf.translation.truncate(), p.team))
        .collect();
    let mut gone: std::collections::HashSet<Entity> = std::collections::HashSet::new();
    let cancel_dist = PROJECTILE_RADIUS * 2.0;
    for i in 0..entries.len() {
        if gone.contains(&entries[i].0) {
            continue;
        }
        for j in (i + 1)..entries.len() {
            if gone.contains(&entries[j].0) {
                continue;
            }
            if entries[i].2 == entries[j].2 {
                continue; // same team — pass through
            }
            if entries[i].1.distance(entries[j].1) < cancel_dist {
                gone.insert(entries[i].0);
                gone.insert(entries[j].0);
                break;
            }
        }
    }
    for e in gone {
        commands.entity(e).despawn();
    }
}

fn projectile_vs_walls(
    mut commands: Commands,
    projectiles: Query<(Entity, &Transform), With<Projectile>>,
    walls: Query<(&Transform, &Wall)>,
) {
    for (pe, ptf) in &projectiles {
        let ppos = ptf.translation.truncate();
        for (wtf, wall) in &walls {
            let wc = wtf.translation.truncate();
            let diff = ppos - wc;
            let clamped = Vec2::new(
                diff.x.clamp(-wall.half_extents.x, wall.half_extents.x),
                diff.y.clamp(-wall.half_extents.y, wall.half_extents.y),
            );
            let closest = wc + clamped;
            let d = (ppos - closest).length();
            if d < PROJECTILE_RADIUS {
                commands.entity(pe).despawn();
                break;
            }
        }
    }
}

fn projectile_vs_ships(
    mut commands: Commands,
    projectiles: Query<(Entity, &Transform, &Projectile)>,
    ships: Query<(Entity, &Transform, &Ship), (With<Tagable>, Without<Respawning>)>,
    mut flags: Query<&mut Flag>,
    respawn_points: Query<&RespawnPoint>,
    mut visibilities: Query<&mut Visibility>,
) {
    let hit_dist = SHIP_RADIUS + PROJECTILE_RADIUS;
    let mut downed: std::collections::HashSet<Entity> = std::collections::HashSet::new();
    let mut spent: std::collections::HashSet<Entity> = std::collections::HashSet::new();

    for (pe, ptf, proj) in &projectiles {
        let ppos = ptf.translation.truncate();
        for (se, stf, ship) in &ships {
            if ship.team == proj.team || downed.contains(&se) {
                continue;
            }
            let spos = stf.translation.truncate();
            if spos.distance(ppos) < hit_dist {
                // Drop any flag this ship carries.
                for mut flag in flags.iter_mut() {
                    if matches!(flag.state, FlagState::Carried(c) if c == se) {
                        flag.state = FlagState::Dropped(spos);
                    }
                }
                commands.entity(se).remove::<CarryingFlag>();
                if let Ok(rp) = respawn_points.get(se) {
                    commands.entity(se).insert(Respawning {
                        timer: Timer::from_seconds(RESPAWN_SECS, TimerMode::Once),
                        point: rp.0,
                    });
                    if let Ok(mut vis) = visibilities.get_mut(se) {
                        *vis = Visibility::Hidden;
                    }
                }
                downed.insert(se);
                spent.insert(pe);
                break;
            }
        }
    }

    for pe in spent {
        commands.entity(pe).despawn();
    }
}
