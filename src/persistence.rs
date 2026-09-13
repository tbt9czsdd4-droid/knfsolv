use crate::{Result, solver::Settings, state::Position};
use serde::{Deserialize, Serialize};
use std::{io::Write, path::Path};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Session {
    pub position: Position,
    #[serde(default)]
    pub settings: Settings,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desktop: Option<crate::game::DesktopGame>,
}
impl Session {
    pub fn new(n: usize) -> Self {
        Self {
            position: Position::new(n),
            settings: Settings::default(),
            desktop: None,
        }
    }
    pub fn validate(&self) -> Result<()> {
        self.position.validate()?;
        self.settings.validate(&self.position)?;
        if let Some(desktop) = &self.desktop {
            desktop.validate(self)?;
        }
        Ok(())
    }
    pub fn load(path: &Path) -> Result<Self> {
        let s: Self = serde_json::from_reader(std::fs::File::open(path)?)?;
        s.validate()?;
        Ok(s)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        save_json(path, self)
    }
}
pub fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer_pretty(&mut tmp, value)?;
    writeln!(tmp)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}

/// GUI saves contain position context only; explicit CLI/TUI session settings still round-trip.
pub fn desktop_value(session: &Session) -> serde_json::Value {
    let mut value = serde_json::to_value(session).expect("serializable session");
    value["settings"] = serde_json::json!({"perspective": session.settings.perspective});
    if let Some(desktop) = value.get_mut("desktop").and_then(|v| v.as_object_mut()) {
        desktop.remove("advisor");
    }
    value
}
pub fn save_desktop(session: &Session, path: &Path) -> Result<()> {
    session.validate()?;
    save_json(path, &desktop_value(session))
}
