use bevy::prelude::*;
use bevy::ui::FocusPolicy;

use crate::bot::{AllyBot, AllyMode, BotDifficulty, BotNumber};
use crate::flag::Score;
use crate::input::SprintButtonText;
use crate::player::{PlayerControlled, Ship, Stamina};
use crate::projectile::GameMode;
use crate::team::Team;
use crate::GameSet;

#[derive(Component)]
pub struct ScoreText;

#[derive(Component)]
pub struct StaminaFill;

#[derive(Component)]
pub struct BotHudChip {
    pub bot: Entity,
    pub kind: ChipKind,
}

#[derive(Clone, Copy, PartialEq)]
pub enum ChipKind {
    Mode,
    Difficulty,
}

#[derive(Component)]
pub struct ChipText;

#[derive(Component)]
pub struct PanelHeader;

#[derive(Component)]
pub struct PanelBody;

#[derive(Component)]
pub struct PanelChevron;

#[derive(Component)]
pub struct ModeToggle;

#[derive(Component)]
pub struct ModeToggleText;

#[derive(Component, Clone, Copy)]
pub struct TeamAdjust {
    pub red: bool,
    pub delta: i32,
}

#[derive(Resource, Default)]
pub struct PanelCollapsed(pub bool);

#[derive(Resource)]
pub struct MatchState {
    pub target_score: u32,
    pub timer: Timer,
    pub winner: Option<Team>,
}

impl Default for MatchState {
    fn default() -> Self {
        Self {
            target_score: 5,
            timer: Timer::from_seconds(240.0, TimerMode::Once),
            winner: None,
        }
    }
}

pub struct HudPlugin;
impl Plugin for HudPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<MatchState>()
            .init_resource::<PanelCollapsed>()
            .add_systems(
                Update,
                (
                    update_score,
                    update_stamina,
                    tick_match,
                    handle_chip_tap,
                    update_chip_text,
                    chip_hover_feedback,
                    handle_panel_toggle,
                    apply_panel_collapsed,
                    handle_mode_toggle,
                    update_mode_toggle_text,
                    update_sprint_button_text,
                    handle_team_adjust,
                )
                    .in_set(GameSet::Hud),
            );
    }
}

pub fn setup_hud(mut commands: Commands) {
    // Top-center score
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(16.0),
                    left: Val::Px(0.0),
                    width: Val::Percent(100.0),
                    justify_content: JustifyContent::Center,
                    ..default()
                },
                ..default()
            },
            crate::game::PlayingEntity,
        ))
        .with_children(|p| {
            p.spawn((
                TextBundle::from_section(
                    "0 - 0",
                    TextStyle { font_size: 42.0, color: Color::WHITE, ..default() },
                ),
                ScoreText,
            ));
        });

    // Game mode toggle — top-left, tappable.
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(20.0),
                    left: Val::Px(20.0),
                    padding: UiRect::all(Val::Px(6.0)),
                    min_width: Val::Px(160.0),
                    ..default()
                },
                background_color: Color::srgba(0.1, 0.1, 0.2, 0.7).into(),
                border_radius: BorderRadius::all(Val::Px(6.0)),
                focus_policy: FocusPolicy::Block,
                ..default()
            },
            Interaction::default(),
            ModeToggle,
            crate::game::PlayingEntity,
        ))
        .with_children(|p| {
            p.spawn((
                TextBundle::from_section(
                    "MODE: CLASSIC",
                    TextStyle { font_size: 14.0, color: Color::WHITE, ..default() },
                ),
                ModeToggleText,
            ));
        });

    // Stamina bar — top-left, below mode toggle
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(54.0),
                    left: Val::Px(20.0),
                    width: Val::Px(220.0),
                    height: Val::Px(14.0),
                    padding: UiRect::all(Val::Px(2.0)),
                    ..default()
                },
                background_color: Color::srgba(0.1, 0.1, 0.2, 0.7).into(),
                ..default()
            },
            crate::game::PlayingEntity,
        ))
        .with_children(|bar| {
            bar.spawn((
                NodeBundle {
                    style: Style {
                        width: Val::Percent(100.0),
                        height: Val::Percent(100.0),
                        ..default()
                    },
                    background_color: Color::srgb(0.4, 0.9, 1.0).into(),
                    ..default()
                },
                StaminaFill,
            ));
        });
}

/// Right-side stacked panel. Header (tappable) collapses the body.
pub fn setup_bot_hud(
    mut commands: Commands,
    bots: Query<(Entity, &Ship, &BotNumber, Option<&AllyBot>), With<BotDifficulty>>,
    endless: Res<crate::game::EndlessMode>,
) {
    let endless = endless.0;
    commands
        .spawn((
            NodeBundle {
                style: Style {
                    position_type: PositionType::Absolute,
                    top: Val::Px(20.0),
                    right: Val::Px(20.0),
                    flex_direction: FlexDirection::Column,
                    padding: UiRect::all(Val::Px(8.0)),
                    ..default()
                },
                background_color: Color::srgba(0.05, 0.05, 0.12, 0.7).into(),
                border_radius: BorderRadius::all(Val::Px(8.0)),
                ..default()
            },
            crate::game::PlayingEntity,
        ))
        .with_children(|panel| {
            // Header — tappable to collapse/expand
            panel
                .spawn((
                    NodeBundle {
                        style: Style {
                            flex_direction: FlexDirection::Row,
                            align_items: AlignItems::Center,
                            justify_content: JustifyContent::SpaceBetween,
                            column_gap: Val::Px(8.0),
                            padding: UiRect::all(Val::Px(4.0)),
                            min_width: Val::Px(120.0),
                            ..default()
                        },
                        focus_policy: FocusPolicy::Block,
                        ..default()
                    },
                    Interaction::default(),
                    PanelHeader,
                ))
                .with_children(|h| {
                    h.spawn(TextBundle::from_section(
                        "BOTS",
                        TextStyle { font_size: 14.0, color: Color::WHITE, ..default() },
                    ));
                    h.spawn((
                        TextBundle::from_section(
                            "-",
                            TextStyle { font_size: 18.0, color: Color::WHITE, ..default() },
                        ),
                        PanelChevron,
                    ));
                });

            // Body — the collapsible part
            panel
                .spawn((
                    NodeBundle {
                        style: Style {
                            flex_direction: FlexDirection::Column,
                            row_gap: Val::Px(6.0),
                            padding: UiRect::top(Val::Px(6.0)),
                            ..default()
                        },
                        ..default()
                    },
                    PanelBody,
                ))
                .with_children(|body| {
                    // Order rows by bot number for stable display.
                    let mut rows: Vec<_> = bots.iter().collect();
                    rows.sort_by_key(|(_, _, n, _)| n.0);
                    for (bot_entity, ship, number, ally) in rows {
                        let is_ally = ally.is_some();
                        // Story mode: only show ally rows — the player can't
                        // change enemy settings mid-match.
                        if !endless && !is_ally {
                            continue;
                        }

                        body.spawn(NodeBundle {
                            style: Style {
                                flex_direction: FlexDirection::Row,
                                align_items: AlignItems::Center,
                                column_gap: Val::Px(8.0),
                                ..default()
                            },
                            ..default()
                        })
                        .with_children(|row| {
                            // Team-color dot
                            row.spawn(NodeBundle {
                                style: Style {
                                    width: Val::Px(12.0),
                                    height: Val::Px(12.0),
                                    ..default()
                                },
                                background_color: ship.team.color().into(),
                                border_radius: BorderRadius::MAX,
                                ..default()
                            });

                            // Bot number
                            row.spawn(NodeBundle {
                                style: Style {
                                    width: Val::Px(22.0),
                                    ..default()
                                },
                                ..default()
                            })
                            .with_children(|c| {
                                c.spawn(TextBundle::from_section(
                                    format!("{}", number.0),
                                    TextStyle {
                                        font_size: 18.0,
                                        color: Color::srgb(1.0, 1.0, 0.4),
                                        ..default()
                                    },
                                ));
                            });

                            // Mode chip (allies only)
                            if is_ally {
                                spawn_chip(row, bot_entity, ChipKind::Mode);
                            } else {
                                row.spawn(NodeBundle {
                                    style: Style {
                                        width: Val::Px(34.0),
                                        height: Val::Px(28.0),
                                        ..default()
                                    },
                                    ..default()
                                });
                            }

                            // Difficulty chip (always)
                            spawn_chip(row, bot_entity, ChipKind::Difficulty);
                        });
                    }

                    // Endless-only: team +/- controls.
                    if endless {
                        spawn_team_adjust_row(body, "Red allies", true);
                        spawn_team_adjust_row(body, "Blue enemies", false);
                    }
                });
        });
}

fn spawn_team_adjust_row(body: &mut ChildBuilder, label: &str, red: bool) {
    body.spawn(NodeBundle {
        style: Style {
            flex_direction: FlexDirection::Row,
            align_items: AlignItems::Center,
            column_gap: Val::Px(8.0),
            padding: UiRect::top(Val::Px(4.0)),
            ..default()
        },
        ..default()
    })
    .with_children(|row| {
        row.spawn(NodeBundle {
            style: Style { width: Val::Px(96.0), ..default() },
            ..default()
        })
        .with_children(|c| {
            c.spawn(TextBundle::from_section(
                label,
                TextStyle { font_size: 13.0, color: Color::WHITE, ..default() },
            ));
        });
        for (text, delta) in [("-", -1), ("+", 1)] {
            row.spawn((
                ButtonBundle {
                    style: Style {
                        width: Val::Px(28.0),
                        height: Val::Px(24.0),
                        justify_content: JustifyContent::Center,
                        align_items: AlignItems::Center,
                        ..default()
                    },
                    background_color: Color::srgba(0.2, 0.2, 0.4, 0.9).into(),
                    border_radius: BorderRadius::all(Val::Px(4.0)),
                    focus_policy: FocusPolicy::Block,
                    ..default()
                },
                TeamAdjust { red, delta },
            ))
            .with_children(|b| {
                b.spawn(TextBundle::from_section(
                    text,
                    TextStyle { font_size: 16.0, color: Color::WHITE, ..default() },
                ));
            });
        }
    });
}

fn handle_team_adjust(
    q: Query<(&Interaction, &TeamAdjust), Changed<Interaction>>,
    mut counts: ResMut<crate::game::BotCounts>,
    mut next: ResMut<NextState<crate::game::AppState>>,
) {
    for (i, adj) in &q {
        if *i == Interaction::Pressed {
            if adj.red {
                counts.red_allies = (counts.red_allies as i32 + adj.delta).clamp(0, 4) as u32;
            } else {
                counts.blue_enemies = (counts.blue_enemies as i32 + adj.delta).clamp(1, 4) as u32;
            }
            next.set(crate::game::AppState::Restarting);
        }
    }
}

fn spawn_chip(row: &mut ChildBuilder, bot: Entity, kind: ChipKind) {
    row.spawn((
        NodeBundle {
            style: Style {
                width: Val::Px(34.0),
                height: Val::Px(28.0),
                justify_content: JustifyContent::Center,
                align_items: AlignItems::Center,
                ..default()
            },
            background_color: Color::srgba(0.2, 0.2, 0.4, 0.85).into(),
            border_radius: BorderRadius::all(Val::Px(5.0)),
            focus_policy: FocusPolicy::Block,
            ..default()
        },
        Interaction::default(),
        BotHudChip { bot, kind },
    ))
    .with_children(|c| {
        c.spawn((
            TextBundle::from_section(
                "?",
                TextStyle { font_size: 18.0, color: Color::WHITE, ..default() },
            ),
            ChipText,
        ));
    });
}

fn update_score(
    score: Res<Score>,
    _state: Res<MatchState>,
    mut q: Query<&mut Text, With<ScoreText>>,
) {
    for mut text in &mut q {
        text.sections[0].value = format!("{} - {}", score.red, score.blue);
    }
}

fn update_stamina(
    player: Query<&Stamina, With<PlayerControlled>>,
    mut fills: Query<&mut Style, With<StaminaFill>>,
) {
    let Ok(stamina) = player.get_single() else { return };
    for mut style in &mut fills {
        style.width = Val::Percent((stamina.current / stamina.max * 100.0).max(0.0));
    }
}

fn tick_match(
    time: Res<Time>,
    score: Res<Score>,
    mut state: ResMut<MatchState>,
) {
    if state.winner.is_some() { return; }
    state.timer.tick(time.delta());
    if score.red >= state.target_score {
        state.winner = Some(Team::Red);
    } else if score.blue >= state.target_score {
        state.winner = Some(Team::Blue);
    } else if state.timer.finished() {
        state.winner = Some(if score.red >= score.blue { Team::Red } else { Team::Blue });
    }
}

fn handle_chip_tap(
    q: Query<(&Interaction, &BotHudChip), Changed<Interaction>>,
    mut diffs: Query<&mut BotDifficulty>,
    mut modes: Query<&mut AllyMode>,
) {
    for (interaction, chip) in &q {
        if *interaction != Interaction::Pressed {
            continue;
        }
        match chip.kind {
            ChipKind::Mode => {
                if let Ok(mut m) = modes.get_mut(chip.bot) {
                    *m = m.next();
                }
            }
            ChipKind::Difficulty => {
                if let Ok(mut d) = diffs.get_mut(chip.bot) {
                    *d = d.next();
                }
            }
        }
    }
}

fn update_chip_text(
    chips: Query<&BotHudChip>,
    mut texts: Query<(&Parent, &mut Text), With<ChipText>>,
    diffs: Query<&BotDifficulty>,
    modes: Query<&AllyMode>,
) {
    for (parent, mut text) in &mut texts {
        let Ok(chip) = chips.get(parent.get()) else { continue };
        let label = match chip.kind {
            ChipKind::Mode => match modes.get(chip.bot).copied() {
                Ok(AllyMode::Auto) => "A",
                Ok(AllyMode::Offense) => "O",
                Ok(AllyMode::Defense) => "D",
                _ => "?",
            },
            ChipKind::Difficulty => match diffs.get(chip.bot).copied() {
                Ok(BotDifficulty::Easy) => "e",
                Ok(BotDifficulty::Medium) => "m",
                Ok(BotDifficulty::Hard) => "h",
                _ => "?",
            },
        };
        text.sections[0].value = label.to_string();
    }
}

fn chip_hover_feedback(
    mut q: Query<(&Interaction, &mut BackgroundColor), (Changed<Interaction>, With<BotHudChip>)>,
) {
    for (i, mut bg) in &mut q {
        bg.0 = match *i {
            Interaction::Pressed => Color::srgba(0.4, 0.4, 0.7, 0.95),
            Interaction::Hovered => Color::srgba(0.3, 0.3, 0.55, 0.9),
            Interaction::None => Color::srgba(0.2, 0.2, 0.4, 0.85),
        };
    }
}

fn handle_panel_toggle(
    q: Query<&Interaction, (Changed<Interaction>, With<PanelHeader>)>,
    mut collapsed: ResMut<PanelCollapsed>,
) {
    for i in &q {
        if *i == Interaction::Pressed {
            collapsed.0 = !collapsed.0;
        }
    }
}

fn apply_panel_collapsed(
    collapsed: Res<PanelCollapsed>,
    mut body: Query<&mut Style, With<PanelBody>>,
    mut chevron: Query<&mut Text, With<PanelChevron>>,
) {
    if !collapsed.is_changed() {
        return;
    }
    for mut style in &mut body {
        style.display = if collapsed.0 {
            Display::None
        } else {
            Display::Flex
        };
    }
    for mut text in &mut chevron {
        text.sections[0].value = if collapsed.0 { "+".into() } else { "-".into() };
    }
}

fn handle_mode_toggle(
    q: Query<&Interaction, (Changed<Interaction>, With<ModeToggle>)>,
    mut mode: ResMut<GameMode>,
) {
    for i in &q {
        if *i == Interaction::Pressed {
            *mode = mode.next();
        }
    }
}

fn update_mode_toggle_text(mode: Res<GameMode>, mut q: Query<&mut Text, With<ModeToggleText>>) {
    for mut text in &mut q {
        text.sections[0].value = format!("MODE: {}", mode.label());
    }
}

fn update_sprint_button_text(mode: Res<GameMode>, mut q: Query<&mut Text, With<SprintButtonText>>) {
    for mut text in &mut q {
        text.sections[0].value = match *mode {
            GameMode::Classic => "BOOST".into(),
            GameMode::Shooter => "FIRE".into(),
        };
    }
}
