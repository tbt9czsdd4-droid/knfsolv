use crate::{Result, solver::Settings, state::Position};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    /// Parses and validates a saved session, wherever it was stored.
    pub fn from_json(json: &str) -> Result<Self> {
        let s: Self = serde_json::from_str(json)?;
        s.validate()?;
        Ok(s)
    }
    pub fn load(path: &Path) -> Result<Self> {
        Self::from_json(&std::fs::read_to_string(path)?)
    }
    pub fn save(&self, path: &Path) -> Result<()> {
        self.validate()?;
        save_json(path, self)
    }
}
/// Where one JSON document is kept: a file natively, browser storage on the web.
pub trait Storage {
    /// `Ok(None)` when nothing has been stored yet.
    fn read(&self) -> Result<Option<String>>;
    fn write(&self, contents: &str) -> Result<()>;
}
/// A file that is replaced atomically; missing parent directories are created on write.
pub struct FileStorage(pub PathBuf);
impl Storage for FileStorage {
    fn read(&self) -> Result<Option<String>> {
        match std::fs::read_to_string(&self.0) {
            Ok(contents) => Ok(Some(contents)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
    fn write(&self, contents: &str) -> Result<()> {
        if let Some(parent) = self.0.parent() {
            std::fs::create_dir_all(parent)?;
        }
        write_atomic(&self.0, contents)
    }
}
/// Keeps the document only while the program runs.
#[derive(Default)]
pub struct MemoryStorage(std::cell::RefCell<Option<String>>);
impl Storage for MemoryStorage {
    fn read(&self) -> Result<Option<String>> {
        Ok(self.0.borrow().clone())
    }
    fn write(&self, contents: &str) -> Result<()> {
        *self.0.borrow_mut() = Some(contents.into());
        Ok(())
    }
}

/// Pretty JSON with a trailing newline, as every saved document is written.
pub fn to_json(value: &impl Serialize) -> Result<String> {
    let mut json = serde_json::to_string_pretty(value)?;
    json.push('\n');
    Ok(json)
}
pub fn save_json(path: &Path, value: &impl Serialize) -> Result<()> {
    write_atomic(path, &to_json(value)?)
}
#[cfg(not(target_arch = "wasm32"))]
fn write_atomic(path: &Path, contents: &str) -> Result<()> {
    use std::io::Write;
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let mut tmp = tempfile::NamedTempFile::new_in(parent)?;
    tmp.write_all(contents.as_bytes())?;
    tmp.as_file().sync_all()?;
    tmp.persist(path).map_err(|e| e.error)?;
    Ok(())
}
/// Browsers have no files; they keep documents in a [`Storage`] instead.
#[cfg(target_arch = "wasm32")]
fn write_atomic(_path: &Path, _contents: &str) -> Result<()> {
    Err(crate::Error::Invalid(
        "Saving files is not available in the browser.".into(),
    ))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_storage_reports_missing_documents_and_round_trips() {
        let dir = tempfile::tempdir().unwrap();
        let storage = FileStorage(dir.path().join("nested/session.json"));
        assert_eq!(storage.read().unwrap(), None);
        let session = Session::new(3);
        storage.write(&to_json(&session).unwrap()).unwrap();
        let stored = storage.read().unwrap().unwrap();
        assert!(stored.ends_with('\n'));
        assert_eq!(Session::from_json(&stored).unwrap(), session);
        assert_eq!(Session::load(&storage.0).unwrap(), session);
    }

    #[test]
    fn invalid_sessions_are_rejected_when_parsed() {
        let mut session = Session::new(2);
        session.settings.perspective = 5;
        let json = to_json(&session).unwrap();
        assert!(Session::from_json(&json).is_err());
        assert!(Session::from_json("{}").is_err());
    }
}
