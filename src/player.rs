use bevy::prelude::*;
use bevy::sprite::{MaterialMesh2dBundle, Mesh2dHandle};

use crate::bot::{
    AllyBot, AllyMode, BotDifficulty, BotNumber, BotNumberLabel, BotRole, LabelFor, LabelOffset,
};
use crate::projectile::FireCooldown;
use crate::movement::{MaxSpeed, Velocity};
use crate::physics::Collider;
use crate::tag::{RespawnPoint, Tagable};
use crate::team::Team;
use crate::GameSet;

pub const SHIP_SIZE: f32 = 22.0;
pub const SHIP_RADIUS: f32 = 14.0;

#[derive(Component)]
pub struct Ship {
    pub team: Team,
}

#[derive(Component)]
pub struct PlayerControlled;

#[derive(Component)]
pub struct Facing(pub f32);

#[derive(Component)]
pub struct Stamina {
    pub current: f32,
    pub max: f32,
    pub regen: f32,
}

impl Default for Stamina {
    fn default() -> Self {
        Self { current: 1.0, max: 1.0, regen: 0.2 }
    }
}

#[derive(Component, Default)]
pub struct Thrusting(pub bool);

pub struct PlayerPlugin;
impl Plugin for PlayerPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, regen_stamina.in_set(GameSet::Gameplay));
    }
}

pub fn spawn_ships(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    difficulty: Res<crate::game::SelectedDifficulty>,
    counts: Res<crate::game::BotCounts>,
) {
    let chosen_diff = difficulty.0;
    let ship_mesh = meshes.add(Triangle2d::new(
        Vec2::new(SHIP_SIZE, 0.0),
        Vec2::new(-SHIP_SIZE * 0.6, SHIP_SIZE * 0.6),
        Vec2::new(-SHIP_SIZE * 0.6, -SHIP_SIZE * 0.6),
    ));

    // Player always on Red.
    spawn_ship(&mut commands, &mut materials, ship_mesh.clone(), Team::Red, Vec2::new(-600.0, 0.0), ShipKind::Player, chosen_diff);

    // Red allies
    let mut number: u32 = 1;
    for i in 0..counts.red_allies {
        let y = 60.0 + (i as f32) * 60.0;
        spawn_ship(
            &mut commands,
            &mut materials,
            ship_mesh.clone(),
            Team::Red,
            Vec2::new(-600.0, y),
            ShipKind::Ally(BotRole::Guardian, number),
            chosen_diff,
        );
        number += 1;
    }

    // Blue enemies — alternate Grabber / Guardian so both roles are filled.
    for i in 0..counts.blue_enemies {
        let y = (i as f32 - (counts.blue_enemies as f32 - 1.0) * 0.5) * 80.0;
        let role = if i % 2 == 0 { BotRole::Grabber } else { BotRole::Guardian };
        spawn_ship(
            &mut commands,
            &mut materials,
            ship_mesh.clone(),
            Team::Blue,
            Vec2::new(600.0, y),
            ShipKind::Enemy(role, number),
            chosen_diff,
        );
        number += 1;
    }
}

enum ShipKind {
    Player,
    Ally(BotRole, u32),
    Enemy(BotRole, u32),
}

fn spawn_ship(
    commands: &mut Commands,
    materials: &mut ResMut<Assets<ColorMaterial>>,
    mesh: Handle<Mesh>,
    team: Team,
    pos: Vec2,
    kind: ShipKind,
    difficulty: BotDifficulty,
) {
    let facing = if matches!(team, Team::Red) { 0.0 } else { std::f32::consts::PI };
    let ship_entity = {
        let mut e = commands.spawn((
            MaterialMesh2dBundle {
                mesh: Mesh2dHandle(mesh),
                material: materials.add(team.color()),
                transform: Transform::from_translation(pos.extend(5.0))
                    .with_rotation(Quat::from_rotation_z(facing)),
                ..default()
            },
            Ship { team },
            Velocity(Vec2::ZERO),
            MaxSpeed(320.0),
            Facing(facing),
            Stamina::default(),
            Thrusting(false),
            Collider::Circle { radius: SHIP_RADIUS },
            Tagable,
            RespawnPoint(pos),
            FireCooldown::default(),
            crate::game::PlayingEntity,
        ));
        let bot_number = match kind {
            ShipKind::Player => { e.insert(PlayerControlled); None }
            ShipKind::Ally(role, n) => {
                e.insert((role, AllyBot, AllyMode::default(), difficulty, BotNumber(n)));
                Some(n)
            }
            ShipKind::Enemy(role, n) => {
                e.insert((role, difficulty, BotNumber(n)));
                Some(n)
            }
        };
        let _ = bot_number;
        e.id()
    };

    let n = match kind {
        ShipKind::Player => return,
        ShipKind::Ally(_, n) | ShipKind::Enemy(_, n) => n,
    };

    // Dark shadow under + bright foreground for fake-bold outline effect.
    commands.spawn((
        Text2dBundle {
            text: Text::from_section(
                format!("{}", n),
                TextStyle { font_size: 42.0, color: Color::srgba(0.0, 0.0, 0.0, 0.95), ..default() },
            ),
            transform: Transform::from_translation((pos + Vec2::new(2.0, -2.0)).extend(9.5)),
            ..default()
        },
        BotNumberLabel,
        LabelFor(ship_entity),
        LabelOffset(Vec2::new(2.0, -2.0)),
        crate::game::PlayingEntity,
    ));
    commands.spawn((
        Text2dBundle {
            text: Text::from_section(
                format!("{}", n),
                TextStyle { font_size: 42.0, color: Color::srgb(1.0, 1.0, 0.4), ..default() },
            ),
            transform: Transform::from_translation(pos.extend(10.0)),
            ..default()
        },
        BotNumberLabel,
        LabelFor(ship_entity),
        crate::game::PlayingEntity,
    ));
}

fn regen_stamina(time: Res<Time>, mut q: Query<(&mut Stamina, &Thrusting)>) {
    for (mut s, thrusting) in &mut q {
        if !thrusting.0 {
            s.current = (s.current + s.regen * time.delta_seconds()).min(s.max);
        }
    }
}
