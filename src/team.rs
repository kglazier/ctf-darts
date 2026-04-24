use bevy::prelude::*;

#[derive(Component, Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Team {
    Red,
    Blue,
}

impl Team {
    pub fn color(self) -> Color {
        match self {
            Team::Red => Color::srgb(1.0, 0.3, 0.4),
            Team::Blue => Color::srgb(0.3, 0.7, 1.0),
        }
    }

    pub fn dim_color(self) -> Color {
        match self {
            Team::Red => Color::srgba(1.0, 0.3, 0.4, 0.18),
            Team::Blue => Color::srgba(0.3, 0.7, 1.0, 0.18),
        }
    }

    pub fn opposite(self) -> Team {
        match self {
            Team::Red => Team::Blue,
            Team::Blue => Team::Red,
        }
    }
}
