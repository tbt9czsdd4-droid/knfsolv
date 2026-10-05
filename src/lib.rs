pub mod coaching;
pub mod dice;
pub mod game;
#[cfg(feature = "gui")]
pub mod gui;
pub mod persistence;
pub mod policy;
pub mod preferences;
pub mod rules;
pub mod solver;
pub mod state;
#[cfg(not(target_arch = "wasm32"))]
pub mod tui;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("{0}")]
    Invalid(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error(transparent)]
    Json(#[from] serde_json::Error),
}
pub type Result<T> = std::result::Result<T, Error>;
