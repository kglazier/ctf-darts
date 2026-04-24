use bevy::prelude::*;
use bevy::ui::FocusPolicy;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;

use crate::bot::BotDifficulty;
use crate::flag::Score;
use crate::hud::MatchState;
use crate::GameSet;

#[derive(States, Default, Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppState {
    #[default]
    Menu,
    Playing,
    GameOver,
    /// Transient state: cleans up and auto-transitions back to Playing so
    /// OnEnter(Playing) re-runs the spawn chain.
    Restarting,
}

/// Resource-gated pause — leaves entities alive but freezes gameplay.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub struct Paused(pub bool);

/// Endless mode has no score target — play as long as you want.
/// Derived from `ScoreTarget` at match start; used by the HUD to expose
/// bot +/- controls and all per-bot rows.
#[derive(Resource, Default, Clone, Copy, PartialEq, Eq)]
pub struct EndlessMode(pub bool);

/// Menu-selected winning score. `None` = unlimited (freeplay).
#[derive(Resource, Clone, Copy)]
pub struct ScoreTarget(pub Option<u32>);
impl Default for ScoreTarget {
    fn default() -> Self { Self(Some(5)) }
}

/// Team bot counts. In Story mode these are reset to 1/2. In Endless they
/// can be bumped by HUD +/- buttons and persist across restarts.
#[derive(Resource, Clone, Copy)]
pub struct BotCounts {
    pub red_allies: u32,
    pub blue_enemies: u32,
}
impl Default for BotCounts {
    fn default() -> Self {
        Self { red_allies: 1, blue_enemies: 2 }
    }
}

/// Run-condition used by the gameplay SystemSets.
pub fn playing_unpaused(state: Res<State<AppState>>, pause: Res<Paused>) -> bool {
    matches!(state.get(), AppState::Playing) && !pause.0
}

/// Entities spawned for an active match. Despawned on OnExit(Playing).
#[derive(Component)]
pub struct PlayingEntity;

/// Entities that live during Menu only.
#[derive(Component)]
pub struct MenuEntity;

/// Entities that live during GameOver only.
#[derive(Component)]
pub struct GameOverEntity;

#[derive(Resource, Clone, Copy, Debug)]
pub struct SelectedLevel(pub usize);
impl Default for SelectedLevel {
    fn default() -> Self { Self(0) }
}

#[derive(Resource, Clone, Copy, Debug, Default)]
pub struct SelectedDifficulty(pub BotDifficulty);

#[derive(Resource, Serialize, Deserialize, Debug, Default)]
pub struct Progress {
    pub best: HashMap<usize, BotDifficulty>,
}

impl Progress {
    pub fn record(&mut self, level: usize, diff: BotDifficulty) {
        let better = match self.best.get(&level).copied() {
            None => true,
            Some(BotDifficulty::Easy) => !matches!(diff, BotDifficulty::Easy),
            Some(BotDifficulty::Medium) => matches!(diff, BotDifficulty::Hard),
            Some(BotDifficulty::Hard) => false,
        };
        if better {
            self.best.insert(level, diff);
            save_progress(self);
        }
    }
    pub fn label(&self, level: usize) -> &'static str {
        match self.best.get(&level) {
            Some(BotDifficulty::Easy) => "EASY",
            Some(BotDifficulty::Medium) => "MEDIUM",
            Some(BotDifficulty::Hard) => "HARD",
            None => "-",
        }
    }
}

/// Level definition — wall layout plus a display name.
#[derive(Clone, Copy)]
pub struct WallSpec {
    pub x: f32,
    pub y: f32,
    pub half_w: f32,
    pub half_h: f32,
}

pub struct LevelDef {
    pub name: &'static str,
    pub walls: &'static [WallSpec],
}

pub const LEVELS: [LevelDef; 5] = [
    LevelDef {
        name: "Open Field",
        walls: &[],
    },
    LevelDef {
        name: "Central Divide",
        walls: &[WallSpec { x: 0.0, y: 0.0, half_w: 10.0, half_h: 200.0 }],
    },
    LevelDef {
        name: "Crossroads",
        walls: &[
            WallSpec { x: 0.0, y: 200.0, half_w: 220.0, half_h: 10.0 },
            WallSpec { x: 0.0, y: -200.0, half_w: 220.0, half_h: 10.0 },
            WallSpec { x: -340.0, y: 0.0, half_w: 10.0, half_h: 130.0 },
            WallSpec { x: 340.0, y: 0.0, half_w: 10.0, half_h: 130.0 },
        ],
    },
    LevelDef {
        name: "Four Pillars",
        walls: &[
            WallSpec { x: -260.0, y: 150.0, half_w: 40.0, half_h: 40.0 },
            WallSpec { x: 260.0, y: 150.0, half_w: 40.0, half_h: 40.0 },
            WallSpec { x: -260.0, y: -150.0, half_w: 40.0, half_h: 40.0 },
            WallSpec { x: 260.0, y: -150.0, half_w: 40.0, half_h: 40.0 },
            WallSpec { x: 0.0, y: 0.0, half_w: 120.0, half_h: 10.0 },
        ],
    },
    LevelDef {
        name: "Gauntlet",
        walls: &[
            // Shorter staggered walls with clear gaps so bot routing doesn't
            // deadlock on overlapping wall-avoidance fields.
            WallSpec { x: -160.0, y: 180.0, half_w: 150.0, half_h: 10.0 },
            WallSpec { x: 160.0, y: -180.0, half_w: 150.0, half_h: 10.0 },
            WallSpec { x: 0.0, y: 0.0, half_w: 10.0, half_h: 110.0 },
        ],
    },
];

pub struct GamePlugin;
impl Plugin for GamePlugin {
    fn build(&self, app: &mut App) {
        app.init_state::<AppState>()
            .init_resource::<SelectedLevel>()
            .init_resource::<SelectedDifficulty>()
            .init_resource::<Paused>()
            .init_resource::<EndlessMode>()
            .init_resource::<ScoreTarget>()
            .init_resource::<BotCounts>()
            .insert_resource(load_progress())
            .add_systems(Startup, (setup_camera, on_startup))
            .add_systems(OnEnter(AppState::Menu), spawn_menu)
            .add_systems(OnExit(AppState::Menu), cleanup::<MenuEntity>)
            .add_systems(
                OnEnter(AppState::Playing),
                (
                    enter_playing,
                    crate::arena::spawn_arena,
                    crate::player::spawn_ships,
                    crate::input::spawn_joystick_ui,
                    crate::hud::setup_hud,
                    spawn_pause_button,
                    crate::flag::spawn_flags,
                    crate::hud::setup_bot_hud,
                )
                    .chain(),
            )
            .add_systems(OnExit(AppState::Playing), (cleanup::<PlayingEntity>, unpause))
            .add_systems(OnEnter(AppState::GameOver), spawn_gameover)
            .add_systems(OnExit(AppState::GameOver), cleanup::<GameOverEntity>)
            .add_systems(OnEnter(AppState::Restarting), restart_tick)
            .add_systems(
                Update,
                detect_match_end
                    .in_set(GameSet::Hud)
                    .run_if(in_state(AppState::Playing)),
            )
            .add_systems(Update, (menu_button_clicks).run_if(in_state(AppState::Menu)))
            .add_systems(
                Update,
                (gameover_button_clicks).run_if(in_state(AppState::GameOver)),
            )
            .add_systems(
                Update,
                (
                    pause_button_click,
                    pause_overlay_sync,
                    pause_overlay_clicks,
                )
                    .run_if(in_state(AppState::Playing)),
            );
    }
}

fn setup_camera(mut commands: Commands) {
    use bevy::render::camera::ScalingMode;
    let mut bundle = Camera2dBundle::default();
    bundle.projection.scaling_mode = ScalingMode::AutoMin {
        min_width: crate::ARENA_WIDTH + 80.0,
        min_height: crate::ARENA_HEIGHT + 80.0,
    };
    commands.spawn(bundle);
}

fn on_startup() {
    // Could run first-time setup here; currently no-op.
}

fn cleanup<T: Component>(mut commands: Commands, q: Query<Entity, With<T>>) {
    for e in &q {
        commands.entity(e).despawn_recursive();
    }
}

fn enter_playing(
    mut match_state: ResMut<MatchState>,
    mut score: ResMut<Score>,
    mut pause: ResMut<Paused>,
    target: Res<ScoreTarget>,
    mut endless: ResMut<EndlessMode>,
    mut counts: ResMut<BotCounts>,
) {
    *score = Score::default();
    *match_state = MatchState::default();
    match target.0 {
        Some(n) => {
            match_state.target_score = n;
            endless.0 = false;
        }
        None => {
            match_state.target_score = u32::MAX;
            endless.0 = true;
        }
    }
    pause.0 = false;
    if !endless.0 {
        *counts = BotCounts::default();
    }
}

fn unpause(mut pause: ResMut<Paused>) {
    pause.0 = false;
}

fn restart_tick(mut next: ResMut<NextState<AppState>>) {
    next.set(AppState::Playing);
}

#[derive(Component)]
struct PauseButton;
#[derive(Component)]
struct PauseOverlay;
#[derive(Component)]
struct PauseRestart;
#[derive(Component)]
struct PauseExit;

fn spawn_pause_button(mut commands: Commands) {
    commands.spawn((
        ButtonBundle {
            style: Style {
                position_type: PositionType::Absolute,
                top: Val::Px(20.0),
                left: Val::Percent(50.0),
                margin: UiRect {
                    left: Val::Px(-28.0),
                    top: Val::Px(52.0),
                    ..default()
                },
                width: Val::Px(56.0),
                height: Val::Px(30.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            background_color: Color::srgba(0.1, 0.1, 0.22, 0.85).into(),
            border_radius: BorderRadius::all(Val::Px(5.0)),
            focus_policy: FocusPolicy::Block,
            ..default()
        },
        PauseButton,
        PlayingEntity,
    ))
    .with_children(|b| {
        b.spawn(TextBundle::from_section(
            "||",
            TextStyle { font_size: 18.0, color: Color::WHITE, ..default() },
        ));
    });
}

fn pause_button_click(
    q: Query<&Interaction, (Changed<Interaction>, With<PauseButton>)>,
    mut pause: ResMut<Paused>,
) {
    for i in &q {
        if *i == Interaction::Pressed {
            pause.0 = !pause.0;
        }
    }
}

fn pause_overlay_sync(
    pause: Res<Paused>,
    existing: Query<Entity, With<PauseOverlay>>,
    mut commands: Commands,
) {
    if !pause.is_changed() {
        return;
    }
    for e in &existing {
        commands.entity(e).despawn_recursive();
    }
    if !pause.0 {
        return;
    }
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(20.0),
                    ..default()
                },
                background_color: Color::srgba(0.0, 0.0, 0.0, 0.75).into(),
                z_index: ZIndex::Global(100),
                ..default()
            },
            PauseOverlay,
            PlayingEntity,
        ))
        .with_children(|root| {
            root.spawn(TextBundle::from_section(
                "PAUSED",
                TextStyle { font_size: 56.0, color: Color::WHITE, ..default() },
            ));
            root.spawn(NodeBundle {
                style: Style {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(20.0),
                    ..default()
                },
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(140.0),
                            height: Val::Px(48.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.2, 0.35, 0.6, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    PauseRestart,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "RESTART",
                        TextStyle { font_size: 20.0, color: Color::WHITE, ..default() },
                    ));
                });
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(140.0),
                            height: Val::Px(48.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.35, 0.2, 0.5, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    PauseExit,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "EXIT",
                        TextStyle { font_size: 20.0, color: Color::WHITE, ..default() },
                    ));
                });
            });
        });
}

fn pause_overlay_clicks(
    restart: Query<&Interaction, (Changed<Interaction>, With<PauseRestart>)>,
    exit: Query<&Interaction, (Changed<Interaction>, With<PauseExit>)>,
    mut next: ResMut<NextState<AppState>>,
) {
    for i in &restart {
        if *i == Interaction::Pressed {
            next.set(AppState::Restarting);
        }
    }
    for i in &exit {
        if *i == Interaction::Pressed {
            next.set(AppState::Menu);
        }
    }
}

#[derive(Component)]
struct MenuLevelStart { level: usize, difficulty: BotDifficulty }

#[derive(Component)]
struct MenuModeToggle;

#[derive(Component)]
struct MenuModeText;

#[derive(Component)]
struct MenuScoreDecrease;

#[derive(Component)]
struct MenuScoreIncrease;

#[derive(Component)]
struct MenuScoreUnlimited;

#[derive(Component)]
struct MenuScoreText;

fn spawn_menu(
    mut commands: Commands,
    progress: Res<Progress>,
    mode: Res<crate::projectile::GameMode>,
    target: Res<ScoreTarget>,
) {
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(12.0),
                    padding: UiRect::all(Val::Px(20.0)),
                    ..default()
                },
                background_color: Color::srgba(0.02, 0.02, 0.06, 0.95).into(),
                ..default()
            },
            MenuEntity,
        ))
        .with_children(|root| {
            root.spawn(TextBundle::from_section(
                "CTF DARTS",
                TextStyle { font_size: 48.0, color: Color::srgb(1.0, 1.0, 0.4), ..default() },
            ));

            // Mode toggle
            root.spawn((
                ButtonBundle {
                    style: Style {
                        width: Val::Px(240.0),
                        height: Val::Px(36.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    background_color: Color::srgba(0.18, 0.18, 0.32, 0.95).into(),
                    border_radius: BorderRadius::all(Val::Px(5.0)),
                    focus_policy: FocusPolicy::Block,
                    ..default()
                },
                MenuModeToggle,
            ))
            .with_children(|b| {
                b.spawn((
                    TextBundle::from_section(
                        format!("MODE: {}", mode.label()),
                        TextStyle { font_size: 16.0, color: Color::WHITE, ..default() },
                    ),
                    MenuModeText,
                ));
            });

            // Target-score picker. Selecting Unlimited enables freeplay
            // behavior (no win, bot +/- controls visible in-game).
            root.spawn(NodeBundle {
                style: Style {
                    flex_direction: FlexDirection::Row,
                    align_items: AlignItems::Center,
                    column_gap: Val::Px(8.0),
                    padding: UiRect::all(Val::Px(6.0)),
                    ..default()
                },
                background_color: Color::srgba(0.12, 0.12, 0.22, 0.8).into(),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(32.0),
                            height: Val::Px(32.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.25, 0.25, 0.45, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(5.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    MenuScoreDecrease,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "-",
                        TextStyle { font_size: 18.0, color: Color::WHITE, ..default() },
                    ));
                });
                row.spawn(NodeBundle {
                    style: Style {
                        width: Val::Px(180.0),
                        justify_content: JustifyContent::Center,
                        ..default()
                    },
                    ..default()
                })
                .with_children(|c| {
                    c.spawn((
                        TextBundle::from_section(
                            format_score(target.0),
                            TextStyle { font_size: 16.0, color: Color::WHITE, ..default() },
                        ),
                        MenuScoreText,
                    ));
                });
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(32.0),
                            height: Val::Px(32.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.25, 0.25, 0.45, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(5.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    MenuScoreIncrease,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "+",
                        TextStyle { font_size: 18.0, color: Color::WHITE, ..default() },
                    ));
                });
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(120.0),
                            height: Val::Px(32.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.3, 0.2, 0.5, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(5.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    MenuScoreUnlimited,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "UNLIMITED",
                        TextStyle { font_size: 14.0, color: Color::WHITE, ..default() },
                    ));
                });
            });

            root.spawn(TextBundle::from_section(
                "Select a level and difficulty",
                TextStyle { font_size: 16.0, color: Color::srgba(1.0, 1.0, 1.0, 0.6), ..default() },
            ));
            for (idx, level) in LEVELS.iter().enumerate() {
                root.spawn(NodeBundle {
                    style: Style {
                        flex_direction: FlexDirection::Row,
                        align_items: AlignItems::Center,
                        column_gap: Val::Px(10.0),
                        padding: UiRect::all(Val::Px(6.0)),
                        min_width: Val::Px(520.0),
                        ..default()
                    },
                    background_color: Color::srgba(0.08, 0.08, 0.18, 0.8).into(),
                    border_radius: BorderRadius::all(Val::Px(6.0)),
                    ..default()
                })
                .with_children(|row| {
                    row.spawn(NodeBundle {
                        style: Style { width: Val::Px(200.0), ..default() },
                        ..default()
                    })
                    .with_children(|c| {
                        c.spawn(TextBundle::from_section(
                            format!("{}: {}", idx + 1, level.name),
                            TextStyle { font_size: 18.0, color: Color::WHITE, ..default() },
                        ));
                    });
                    row.spawn(NodeBundle {
                        style: Style { width: Val::Px(120.0), ..default() },
                        ..default()
                    })
                    .with_children(|c| {
                        c.spawn(TextBundle::from_section(
                            format!("Best: {}", progress.label(idx)),
                            TextStyle {
                                font_size: 14.0,
                                color: Color::srgba(1.0, 1.0, 0.5, 0.9),
                                ..default()
                            },
                        ));
                    });
                    for (lbl, diff) in [
                        ("E", BotDifficulty::Easy),
                        ("M", BotDifficulty::Medium),
                        ("H", BotDifficulty::Hard),
                    ] {
                        row.spawn((
                            ButtonBundle {
                                style: Style {
                                    width: Val::Px(44.0),
                                    height: Val::Px(32.0),
                                    justify_content: JustifyContent::Center,
                                    align_items: AlignItems::Center,
                                    ..default()
                                },
                                background_color: Color::srgba(0.2, 0.2, 0.4, 0.9).into(),
                                border_radius: BorderRadius::all(Val::Px(5.0)),
                                focus_policy: FocusPolicy::Block,
                                ..default()
                            },
                            MenuLevelStart { level: idx, difficulty: diff },
                        ))
                        .with_children(|b| {
                            b.spawn(TextBundle::from_section(
                                lbl,
                                TextStyle {
                                    font_size: 16.0,
                                    color: Color::WHITE,
                                    ..default()
                                },
                            ));
                        });
                    }
                });
            }
        });
}

pub fn format_score(target: Option<u32>) -> String {
    match target {
        Some(n) => format!("POINTS TO WIN: {}", n),
        None => "POINTS TO WIN: UNLIMITED".into(),
    }
}

fn menu_button_clicks(
    levels_q: Query<(&Interaction, &MenuLevelStart), Changed<Interaction>>,
    mode_q: Query<&Interaction, (Changed<Interaction>, With<MenuModeToggle>)>,
    score_dec_q: Query<&Interaction, (Changed<Interaction>, With<MenuScoreDecrease>)>,
    score_inc_q: Query<&Interaction, (Changed<Interaction>, With<MenuScoreIncrease>)>,
    score_unl_q: Query<&Interaction, (Changed<Interaction>, With<MenuScoreUnlimited>)>,
    mut level: ResMut<SelectedLevel>,
    mut diff: ResMut<SelectedDifficulty>,
    mut target: ResMut<ScoreTarget>,
    mut mode: ResMut<crate::projectile::GameMode>,
    mut mode_text: Query<&mut Text, (With<MenuModeText>, Without<MenuScoreText>)>,
    mut score_text: Query<&mut Text, With<MenuScoreText>>,
    mut next: ResMut<NextState<AppState>>,
) {
    for i in &mode_q {
        if *i == Interaction::Pressed {
            *mode = mode.next();
            for mut t in &mut mode_text {
                t.sections[0].value = format!("MODE: {}", mode.label());
            }
        }
    }
    let mut score_changed = false;
    for i in &score_dec_q {
        if *i == Interaction::Pressed {
            let current = target.0.unwrap_or(5);
            target.0 = Some(current.saturating_sub(1).max(1));
            score_changed = true;
        }
    }
    for i in &score_inc_q {
        if *i == Interaction::Pressed {
            let current = target.0.unwrap_or(5);
            target.0 = Some((current + 1).min(15));
            score_changed = true;
        }
    }
    for i in &score_unl_q {
        if *i == Interaction::Pressed {
            target.0 = match target.0 {
                Some(_) => None,
                None => Some(5),
            };
            score_changed = true;
        }
    }
    if score_changed {
        for mut t in &mut score_text {
            t.sections[0].value = format_score(target.0);
        }
    }
    for (i, start) in &levels_q {
        if *i == Interaction::Pressed {
            level.0 = start.level;
            diff.0 = start.difficulty;
            next.set(AppState::Playing);
        }
    }
}

fn detect_match_end(state: Res<MatchState>, mut next: ResMut<NextState<AppState>>) {
    if state.winner.is_some() {
        next.set(AppState::GameOver);
    }
}


#[derive(Component)]
struct GameOverRetry;
#[derive(Component)]
struct GameOverMenu;

fn spawn_gameover(
    mut commands: Commands,
    score: Res<Score>,
    state: Res<MatchState>,
    level: Res<SelectedLevel>,
    diff: Res<SelectedDifficulty>,
    mut progress: ResMut<Progress>,
) {
    let you_won = matches!(state.winner, Some(crate::team::Team::Red));
    if you_won {
        progress.record(level.0, diff.0);
    }

    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(0.0),
                    left: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    height: Val::Percent(100.0),
                    flex_direction: FlexDirection::Column,
                    align_items: AlignItems::Center,
                    justify_content: JustifyContent::Center,
                    row_gap: Val::Px(16.0),
                    ..default()
                },
                background_color: Color::srgba(0.0, 0.0, 0.0, 0.85).into(),
                ..default()
            },
            GameOverEntity,
        ))
        .with_children(|root| {
            root.spawn(TextBundle::from_section(
                if you_won { "YOU WON" } else { "YOU LOST" },
                TextStyle {
                    font_size: 56.0,
                    color: if you_won {
                        Color::srgb(0.5, 1.0, 0.5)
                    } else {
                        Color::srgb(1.0, 0.5, 0.5)
                    },
                    ..default()
                },
            ));
            root.spawn(TextBundle::from_section(
                format!("{} - {}", score.red, score.blue),
                TextStyle { font_size: 40.0, color: Color::WHITE, ..default() },
            ));
            root.spawn(NodeBundle {
                style: Style {
                    flex_direction: FlexDirection::Row,
                    column_gap: Val::Px(20.0),
                    ..default()
                },
                ..default()
            })
            .with_children(|row| {
                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(140.0),
                            height: Val::Px(48.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.2, 0.35, 0.6, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    GameOverRetry,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "RETRY",
                        TextStyle { font_size: 20.0, color: Color::WHITE, ..default() },
                    ));
                });

                row.spawn((
                    ButtonBundle {
                        style: Style {
                            width: Val::Px(140.0),
                            height: Val::Px(48.0),
                            justify_content: JustifyContent::Center,
                            align_items: AlignItems::Center,
                            ..default()
                        },
                        background_color: Color::srgba(0.35, 0.2, 0.5, 0.95).into(),
                        border_radius: BorderRadius::all(Val::Px(6.0)),
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    GameOverMenu,
                ))
                .with_children(|b| {
                    b.spawn(TextBundle::from_section(
                        "MENU",
                        TextStyle { font_size: 20.0, color: Color::WHITE, ..default() },
                    ));
                });
            });
        });
}

fn gameover_button_clicks(
    retry_q: Query<&Interaction, (Changed<Interaction>, With<GameOverRetry>)>,
    menu_q: Query<&Interaction, (Changed<Interaction>, With<GameOverMenu>)>,
    mut next: ResMut<NextState<AppState>>,
) {
    for i in &retry_q {
        if *i == Interaction::Pressed {
            // Bevy re-runs OnEnter when transitioning to the same state only
            // after visiting another state first.
            next.set(AppState::Playing);
        }
    }
    for i in &menu_q {
        if *i == Interaction::Pressed {
            next.set(AppState::Menu);
        }
    }
}

// ---------- Persistence ----------

fn save_path() -> PathBuf {
    PathBuf::from("progress.json")
}

pub fn load_progress() -> Progress {
    match std::fs::read(save_path()) {
        Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
        Err(_) => Progress::default(),
    }
}

pub fn save_progress(progress: &Progress) {
    if let Ok(bytes) = serde_json::to_vec_pretty(progress) {
        let _ = std::fs::write(save_path(), bytes);
    }
}
