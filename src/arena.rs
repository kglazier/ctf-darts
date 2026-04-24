use bevy::prelude::*;
use bevy::sprite::{MaterialMesh2dBundle, Mesh2dHandle};

use crate::game::{PlayingEntity, SelectedLevel, LEVELS};
use crate::team::Team;
use crate::{ARENA_HEIGHT, ARENA_WIDTH};

pub const BASE_RADIUS: f32 = 70.0;

#[derive(Component)]
pub struct Wall {
    pub half_extents: Vec2,
}

#[derive(Component)]
pub struct Base {
    pub team: Team,
}

pub struct ArenaPlugin;
impl Plugin for ArenaPlugin {
    fn build(&self, _app: &mut App) {
        // Spawn is driven by OnEnter(AppState::Playing) via GamePlugin.
    }
}

pub fn spawn_arena(
    mut commands: Commands,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<ColorMaterial>>,
    selected: Res<SelectedLevel>,
) {
    let red_pos = Vec2::new(-ARENA_WIDTH * 0.4, 0.0);
    let blue_pos = Vec2::new(ARENA_WIDTH * 0.4, 0.0);

    for (team, pos) in [(Team::Red, red_pos), (Team::Blue, blue_pos)] {
        commands.spawn((
            MaterialMesh2dBundle {
                mesh: Mesh2dHandle(meshes.add(Circle::new(BASE_RADIUS))),
                material: materials.add(team.dim_color()),
                transform: Transform::from_translation(pos.extend(-1.0)),
                ..default()
            },
            Base { team },
            PlayingEntity,
        ));
    }

    let wall_thick = 20.0;
    let half_w = ARENA_WIDTH * 0.5;
    let half_h = ARENA_HEIGHT * 0.5;

    // Outer walls — always present.
    spawn_wall(&mut commands, &mut meshes, &mut materials, Vec2::new(0.0, half_h), Vec2::new(half_w, wall_thick * 0.5));
    spawn_wall(&mut commands, &mut meshes, &mut materials, Vec2::new(0.0, -half_h), Vec2::new(half_w, wall_thick * 0.5));
    spawn_wall(&mut commands, &mut meshes, &mut materials, Vec2::new(-half_w, 0.0), Vec2::new(wall_thick * 0.5, half_h));
    spawn_wall(&mut commands, &mut meshes, &mut materials, Vec2::new(half_w, 0.0), Vec2::new(wall_thick * 0.5, half_h));

    // Interior walls per level.
    let level = &LEVELS[selected.0.min(LEVELS.len() - 1)];
    for wall in level.walls.iter() {
        spawn_wall(
            &mut commands,
            &mut meshes,
            &mut materials,
            Vec2::new(wall.x, wall.y),
            Vec2::new(wall.half_w, wall.half_h),
        );
    }
}

fn spawn_wall(
    commands: &mut Commands,
    meshes: &mut ResMut<Assets<Mesh>>,
    materials: &mut ResMut<Assets<ColorMaterial>>,
    pos: Vec2,
    half_extents: Vec2,
) {
    commands.spawn((
        MaterialMesh2dBundle {
            mesh: Mesh2dHandle(meshes.add(Rectangle::new(half_extents.x * 2.0, half_extents.y * 2.0))),
            material: materials.add(Color::srgb(0.35, 0.35, 0.55)),
            transform: Transform::from_translation(pos.extend(0.0)),
            ..default()
        },
        Wall { half_extents },
        PlayingEntity,
    ));
}
