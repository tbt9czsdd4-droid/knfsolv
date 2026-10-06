//! Desktop defaults are independent of manually saved game progress.
use crate::{
    Error, Result,
    game::{AnalysisMode, ComputerGameConfig, LocalGameConfig},
    persistence::Storage,
    solver::Settings,
};
use serde::{Deserialize, Serialize};
#[cfg(not(target_arch = "wasm32"))]
use std::path::PathBuf;
use std::time::Duration;
use web_time::Instant;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Preferences {
    pub version: u32,
    pub theme: Theme,
    pub score_wheel_limits: ScoreWheelLimits,
    #[serde(serialize_with = "serialize_engine")]
    pub engine: Settings,
    pub analysis_mode: AnalysisMode,
    pub analysis_paused: bool,
    pub local: LocalGameConfig,
    pub computer: ComputerGameConfig,
    pub custom_search: bool,
    pub custom_settings: Settings,
    pub analysis_players: usize,
}
fn serialize_engine<S: serde::Serializer>(
    settings: &Settings,
    serializer: S,
) -> std::result::Result<S::Ok, S::Error> {
    let mut value = serde_json::to_value(settings).map_err(serde::ser::Error::custom)?;
    value.as_object_mut().unwrap().remove("perspective");
    value.serialize(serializer)
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Dark,
    Light,
}
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScoreWheelLimits {
    #[default]
    StopAtEnds,
    WrapAround,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            version: 1,
            theme: Theme::System,
            score_wheel_limits: ScoreWheelLimits::default(),
            engine: Settings::default(),
            analysis_mode: AnalysisMode::Continuous,
            analysis_paused: false,
            local: LocalGameConfig::default(),
            computer: ComputerGameConfig::default(),
            custom_search: false,
            custom_settings: Settings {
                method: crate::solver::Method::Mcts,
                ..Default::default()
            },
            analysis_players: 2,
        }
    }
}
impl Preferences {
    /// Phones analyze each position for a limited time (about 3 s or 20,000 games), which
    /// saves battery; „Länger rechnen“ continues a finished analysis.
    pub fn phone() -> Self {
        Self {
            analysis_mode: AnalysisMode::Limited,
            engine: Settings {
                time_ms: 3_000,
                simulations: 20_000,
                ..Settings::default()
            },
            ..Self::default()
        }
    }
    fn validate(&self) -> Result<()> {
        if self.version != 1
            || self.engine.perspective != 0
            || self.engine.opponent_policies.len() > 8
            || !(1..=8).contains(&self.analysis_players)
        {
            return Err(Error::Invalid("Unsupported or invalid preferences".into()));
        }
        let mut engine = self.engine.clone();
        engine.opponent_policies.clear();
        engine.validate(&crate::state::Position::new(2))?;
        self.local.session(0)?;
        self.computer.session(0)?;
        self.custom_settings
            .validate(&crate::state::Position::new(2))?;
        Ok(())
    }
}
/// The desktop preference file; browsers keep preferences in their own storage.
#[cfg(not(target_arch = "wasm32"))]
pub fn path() -> PathBuf {
    #[cfg(target_os = "windows")]
    let base = std::env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base =
        std::env::var_os("HOME").map(|h| PathBuf::from(h).join("Library/Application Support"));
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")));
    base.unwrap_or_else(|| PathBuf::from(".config"))
        .join("knfsolv/preferences.json")
}
pub struct Store {
    storage: Box<dyn Storage>,
    observed: Preferences,
    pending: Option<Instant>,
    persisted: Preferences,
    pub save_failed: bool,
    pub error: Option<String>,
}
impl Store {
    pub fn load(storage: impl Storage + 'static) -> (Self, Preferences) {
        Self::load_with_defaults(storage, Preferences::default())
    }
    pub fn load_with_defaults(
        storage: impl Storage + 'static,
        defaults: Preferences,
    ) -> (Self, Preferences) {
        let loaded = storage.read().and_then(|stored| match stored {
            Some(json) => {
                let p = serde_json::from_str::<Preferences>(&json)?;
                p.validate()?;
                Ok(p)
            }
            None => Ok(defaults.clone()),
        });
        let (preferences, error) = match loaded {
            Ok(p) => (p, None),
            Err(e) => (
                defaults,
                Some(format!(
                    "Preferences could not be loaded; using defaults. {e}"
                )),
            ),
        };
        (
            Self {
                storage: Box::new(storage),
                observed: preferences.clone(),
                persisted: preferences.clone(),
                save_failed: false,
                pending: None,
                error,
            },
            preferences,
        )
    }
    pub fn observe(&mut self, preferences: &Preferences, now: Instant) {
        if self.observed != *preferences {
            self.observed = preferences.clone();
            self.pending =
                (self.observed != self.persisted).then_some(now + Duration::from_millis(300));
        }
    }
    pub fn due(&self) -> Option<Instant> {
        self.pending
    }
    pub fn tick(&mut self, now: Instant) {
        if self.pending.is_some_and(|due| now >= due) {
            self.flush();
        }
    }
    pub fn flush(&mut self) {
        if self.pending.take().is_none() {
            return;
        }
        let result = (|| {
            self.observed.validate()?;
            self.storage
                .write(&crate::persistence::to_json(&self.observed)?)
        })();
        self.save_failed = result.is_err();
        if result.is_ok() {
            self.persisted = self.observed.clone();
        }
        self.error = result
            .err()
            .map(|e| format!("Preferences were not saved. {e}"));
    }
    pub fn retry(&mut self) {
        if !self.save_failed {
            return;
        }
        self.pending = Some(Instant::now());
        self.flush();
    }
}
impl Drop for Store {
    fn drop(&mut self) {
        self.flush();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::persistence::FileStorage;
    #[test]
    fn phone_defaults_are_valid_limited_analysis() {
        let phone = Preferences::phone();
        phone.validate().unwrap();
        assert_eq!(phone.analysis_mode, AnalysisMode::Limited);
        assert_eq!(phone.engine.time_ms, 3_000);
        let json = crate::persistence::to_json(&phone).unwrap();
        assert_eq!(serde_json::from_str::<Preferences>(&json).unwrap(), phone);
    }

    #[test]
    fn score_wheel_preference_defaults_and_persists() {
        let old: Preferences = serde_json::from_str(r#"{"version":1}"#).unwrap();
        assert_eq!(old.score_wheel_limits, ScoreWheelLimits::StopAtEnds);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        let (mut store, mut p) = Store::load(FileStorage(path.clone()));
        p.score_wheel_limits = ScoreWheelLimits::WrapAround;
        store.observe(&p, Instant::now());
        store.flush();
        assert_eq!(
            Store::load(FileStorage(path)).1.score_wheel_limits,
            ScoreWheelLimits::WrapAround
        );
    }

    #[test]
    fn debounce_roundtrip_exit_flush_and_no_unchanged_writes() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config/preferences.json");
        let (mut store, mut p) = Store::load(FileStorage(path.clone()));
        let now = Instant::now();
        store.observe(&p, now);
        store.tick(now + Duration::from_secs(1));
        assert!(!path.exists());
        p.theme = Theme::Light;
        p.engine.max_depth = 9;
        p.engine.opponent_policies = vec![crate::policy::Policy::Greedy; 3];
        p.engine.baseline_model = Some("custom.json".into());
        p.analysis_paused = true;
        p.local.names[0] = "Ada".into();
        p.computer.computer.fast_play = true;
        store.observe(&p, now);
        store.tick(now + Duration::from_millis(299));
        assert!(!path.exists());
        p.engine.seed = 17;
        store.observe(&p, now + Duration::from_millis(200));
        store.tick(now + Duration::from_millis(499));
        assert!(!path.exists());
        store.tick(now + Duration::from_millis(500));
        assert!(store.error.is_none());
        assert_eq!(Store::load(FileStorage(path.clone())).1, p);
        let modified = std::fs::metadata(&path).unwrap().modified().unwrap();
        let bytes = std::fs::read(&path).unwrap();
        store.observe(&p, now);
        store.tick(now + Duration::from_secs(10));
        store.flush();
        assert_eq!(
            std::fs::metadata(&path).unwrap().modified().unwrap(),
            modified
        );
        let mut changed = p.clone();
        changed.theme = Theme::Dark;
        store.observe(&changed, now);
        store.observe(&p, now);
        store.flush();
        assert_eq!(std::fs::read(&path).unwrap(), bytes);
        p.engine.seed = 123;
        store.observe(&p, now);
        drop(store);
        assert_eq!(Store::load(FileStorage(path.clone())).1, p);
        let value: serde_json::Value =
            serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
        assert!(value["engine"].get("perspective").is_none());
        for key in ["position", "dice", "scores", "rng", "undo"] {
            assert!(value.get(key).is_none());
        }
    }
    #[test]
    fn corrupt_and_future_files_are_preserved_until_a_change() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preferences.json");
        for bytes in [
            b"bad json".as_slice(),
            b"{\"version\":999}",
            b"{\"engine\":{\"simulations\":0}}",
        ] {
            std::fs::write(&path, bytes).unwrap();
            let (mut store, mut p) = Store::load(FileStorage(path.clone()));
            assert!(store.error.is_some());
            assert_eq!(p, Preferences::default());
            store.observe(&p, Instant::now());
            store.flush();
            store.retry();
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
            p.theme = Theme::Dark;
            store.observe(&p, Instant::now());
            store.flush();
            assert!(store.error.is_none());
            assert_eq!(Store::load(FileStorage(path.clone())).1, p);
        }
    }
    #[test]
    fn failed_atomic_replacement_retains_file_and_retry_saves_latest_changes() {
        let dir = tempfile::tempdir().unwrap();
        let parent = dir.path().join("blocked");
        std::fs::write(&parent, "obstruction").unwrap();
        let path = parent.join("preferences.json");
        let (mut store, mut p) = Store::load(FileStorage(path.clone()));
        p.engine.seed = 8;
        store.observe(&p, Instant::now());
        store.flush();
        assert!(store.save_failed);
        assert!(store.error.as_ref().unwrap().contains("not saved"));
        assert_eq!(std::fs::read_to_string(&parent).unwrap(), "obstruction");
        std::fs::remove_file(&parent).unwrap();
        store.retry();
        assert!(!store.save_failed);
        assert_eq!(Store::load(FileStorage(path)).1, p);
    }
}
