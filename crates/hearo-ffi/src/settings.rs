//! Module catalog, parameters and the persisted settings record (ADR 0015).
//!
//! Rust owns the meaning of settings: Swift stores the JSON from [`settings_to_json`] and hands
//! it back through [`settings_from_json`]. Reading is forgiving in both directions — unknown
//! modules/params are ignored, missing ones get their defaults, every value is clamped and
//! snapped ([`ParamDesc::snap`]) — so a settings file from an older or newer hEARo still loads.

use hearo_dsp::{ModuleId, ParamDesc, ParamUnit, modules};
use std::collections::BTreeMap;

use hearo_engine::{MAX_PARAMS, MAX_SLOTS};
use serde::{Deserialize, Serialize};

use crate::HearoError;

/// One labelled value of a discrete parameter.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ParamChoiceInfo {
    /// The value to set.
    pub value: f32,
    /// Display label ("Auto", "50 Hz").
    pub label: String,
}

/// One module parameter as a settings UI shows it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ParamInfo {
    /// Stable key (`range`, `threshold`, …).
    pub key: String,
    /// Display name.
    pub name: String,
    /// Unit for display: `"dB"`, `"Hz"`, `"ms"`, `"ratio"`, `"%"` (value is 0..1; show ×100) or `""`.
    pub unit: String,
    /// Minimum value.
    pub min: f32,
    /// Maximum value.
    pub max: f32,
    /// Default value.
    pub default_value: f32,
    /// Slider resolution: values are `min + k·step`; 0 = continuous.
    pub step: f32,
    /// Non-empty = a discrete parameter: only these values exist (show a picker).
    pub choices: Vec<ParamChoiceInfo>,
    /// The module's one "how much is removed" control (the simple UI shows only this one).
    /// Everything else is an advanced setting.
    pub primary: bool,
}

/// One module as a settings UI shows it.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ModuleInfo {
    /// Stable key (`desmack`, …).
    pub key: String,
    /// Display name.
    pub name: String,
    /// One sentence for the UI.
    pub description: String,
    /// A cleanup module (has a gain-reduction row in waveforms). False for the limiter.
    pub cleanup: bool,
    /// Can't be switched off (the limiter): show no toggle.
    pub always_on: bool,
    /// Key of the primary param, if any (same as the param with `primary == true`).
    pub primary_param: Option<String>,
    /// Parameters in order.
    pub params: Vec<ParamInfo>,
}

/// A parameter value in [`EngineSettings`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct ParamValue {
    /// Param key.
    pub key: String,
    /// Value (clamped and snapped when applied).
    pub value: f32,
}

/// One module's settings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct ModuleSetting {
    /// Module key.
    pub key: String,
    /// Enabled (ignored for always-on modules).
    #[serde(default = "yes")]
    pub enabled: bool,
    /// Parameter values; missing ones mean "default".
    #[serde(default)]
    pub params: Vec<ParamValue>,
}

fn yes() -> bool {
    true
}

/// Everything the user can set per module: enabled flags + parameter values. Plain data; persist
/// it with [`settings_to_json`]. Global bypass is not part of it (it's a momentary A/B).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, uniffi::Record)]
pub struct EngineSettings {
    /// Modules (any order; unknown keys ignored, missing modules get defaults).
    #[serde(default)]
    pub modules: Vec<ModuleSetting>,
}

/// Module keys a user sees: the registry minus utilities, in chain order.
pub(crate) fn user_module_keys() -> Vec<String> {
    modules::KEYS
        .iter()
        .filter(|k| ModuleId::from_key(k).is_some_and(|id| id != ModuleId::Utility))
        .map(|k| k.to_string())
        .collect()
}

fn descs(key: &str) -> Vec<ParamDesc> {
    modules::create(key).map(|m| m.params().to_vec()).unwrap_or_default()
}

fn always_on(key: &str) -> bool {
    ModuleId::from_key(key) == Some(ModuleId::Limiter)
}

fn unit(u: ParamUnit) -> &'static str {
    match u {
        ParamUnit::Hz => "Hz",
        ParamUnit::Db => "dB",
        ParamUnit::Ms => "ms",
        ParamUnit::Ratio => "ratio",
        ParamUnit::Percent => "%",
        ParamUnit::None => "",
    }
}

/// Catalog entry for one registry key (`None` for unknown keys).
pub(crate) fn module_info(key: &str) -> Option<ModuleInfo> {
    let m = modules::create(key)?;
    let id = m.id();
    let primary = m.primary_param();
    let params: Vec<ParamInfo> = m
        .params()
        .iter()
        .enumerate()
        .map(|(i, d)| ParamInfo {
            key: d.key.into(),
            name: d.name.into(),
            unit: unit(d.unit).into(),
            min: d.min,
            max: d.max,
            default_value: d.default,
            step: d.step,
            choices: d.choices.iter().map(|c| ParamChoiceInfo { value: c.value, label: c.label.into() }).collect(),
            primary: primary == Some(i),
        })
        .collect();
    Some(ModuleInfo {
        key: key.into(),
        name: id.display_name().into(),
        description: id.description().into(),
        cleanup: ModuleId::CLEANUP.contains(&id),
        always_on: always_on(key),
        primary_param: params.iter().find(|p| p.primary).map(|p| p.key.clone()),
        params,
    })
}

/// Per-slot module state the engine keeps (and re-applies after every rebuild).
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ModuleState {
    pub keys: Vec<String>,
    pub enabled: Vec<bool>,
    /// Snapped parameter values per slot, in the module's param order.
    pub params: Vec<Vec<f32>>,
}

impl ModuleState {
    /// Defaults for these (known) keys.
    pub fn new(keys: Vec<String>) -> Self {
        let params = keys.iter().map(|k| descs(k).iter().map(|d| d.default).collect()).collect();
        Self { enabled: vec![true; keys.len()], keys, params }
    }

    pub fn slot(&self, key: &str) -> Option<usize> {
        self.keys.iter().position(|k| k == key)
    }

    /// Store a parameter (clamped + snapped). Returns `(slot, param index, applied value)`.
    pub fn set_param(&mut self, key: &str, param: &str, value: f32) -> Option<(usize, usize, f32)> {
        let slot = self.slot(key)?;
        let descs = descs(key);
        let p = descs.iter().position(|d| d.key == param)?;
        let v = descs[p].snap(value);
        self.params[slot][p] = v;
        Some((slot, p, v))
    }

    /// Enable/disable (always-on modules stay on). Returns the slot and the applied state.
    pub fn set_enabled(&mut self, key: &str, on: bool) -> Option<(usize, bool)> {
        let slot = self.slot(key)?;
        let on = on || always_on(key);
        self.enabled[slot] = on;
        Some((slot, on))
    }

    pub fn settings(&self) -> EngineSettings {
        EngineSettings {
            modules: self
                .keys
                .iter()
                .enumerate()
                .map(|(i, k)| ModuleSetting {
                    key: k.clone(),
                    enabled: self.enabled[i],
                    params: descs(k)
                        .iter()
                        .zip(&self.params[i])
                        .map(|(d, v)| ParamValue { key: d.key.into(), value: *v })
                        .collect(),
                })
                .collect(),
        }
    }

    /// The same settings for a snapshot sidecar (`hearo dataset eval` replays them).
    pub fn live_settings(&self) -> hearo_engine::snapshots::LiveSettings {
        use hearo_engine::snapshots::{LiveModule, LiveParam, LiveSettings};
        LiveSettings {
            modules: self
                .settings()
                .modules
                .into_iter()
                .map(|m| LiveModule {
                    key: m.key,
                    enabled: m.enabled,
                    params: m.params.into_iter().map(|p| LiveParam { key: p.key, value: p.value }).collect(),
                })
                .collect(),
        }
    }

    /// Replace everything with `s` (unknown keys ignored; missing modules/params → defaults).
    pub fn apply(&mut self, s: &EngineSettings) {
        let fresh = Self::new(self.keys.clone());
        self.enabled = fresh.enabled;
        self.params = fresh.params;
        for i in 0..self.keys.len() {
            let key = self.keys[i].clone();
            // First entry for a key wins.
            let Some(m) = s.modules.iter().find(|m| m.key == key) else { continue };
            self.set_enabled(&key, m.enabled);
            for p in &m.params {
                self.set_param(&key, &p.key, p.value);
            }
        }
    }

    /// Enabled flags and `(slot, param) → value` for the supervisor, which re-applies them to
    /// every new engine (device switch, watchdog rebuild, wake).
    pub fn for_supervisor(&self) -> (Vec<bool>, BTreeMap<(usize, usize), f32>) {
        let mut map = BTreeMap::new();
        for (slot, params) in self.params.iter().enumerate().take(MAX_SLOTS) {
            for (p, v) in params.iter().enumerate().take(MAX_PARAMS) {
                map.insert((slot, p), *v);
            }
        }
        (self.enabled.clone(), map)
    }
}

/// Every user-facing module (chain order) with its parameters, for settings UIs.
#[uniffi::export]
pub fn module_catalog() -> Vec<ModuleInfo> {
    user_module_keys().iter().filter_map(|k| module_info(k)).collect()
}

/// Factory settings for every user-facing module.
#[uniffi::export]
pub fn default_settings() -> EngineSettings {
    ModuleState::new(user_module_keys()).settings()
}

/// Settings → JSON for persisting (stable format, ADR 0015).
#[uniffi::export]
pub fn settings_to_json(settings: EngineSettings) -> String {
    serde_json::to_string(&settings).unwrap_or_else(|_| "{}".into())
}

/// JSON → settings. Unknown fields are ignored, missing ones defaulted; only JSON that isn't an
/// object of the right shape fails. The result is normalised (clamped/snapped, every
/// user-facing module present).
#[uniffi::export]
pub fn settings_from_json(json: String) -> Result<EngineSettings, HearoError> {
    let s: EngineSettings =
        serde_json::from_str(&json).map_err(|e| HearoError::Io { message: format!("bad settings: {e}") })?;
    Ok(normalize(&s))
}

/// `s` applied over the defaults for every user-facing module (what an engine would end up with).
pub(crate) fn normalize(s: &EngineSettings) -> EngineSettings {
    let mut st = ModuleState::new(user_module_keys());
    st.apply(s);
    st.settings()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(s: &EngineSettings, m: &str, p: &str) -> f32 {
        s.modules.iter().find(|x| x.key == m).unwrap().params.iter().find(|x| x.key == p).unwrap().value
    }

    #[test]
    fn catalog_lists_user_modules_with_one_primary_each() {
        let cat = module_catalog();
        let keys: Vec<&str> = cat.iter().map(|m| m.key.as_str()).collect();
        assert_eq!(keys, ["dehum", "deplosive", "declick", "desmack", "deess", "limiter"]);
        for m in &cat {
            let primaries = m.params.iter().filter(|p| p.primary).count();
            if m.cleanup {
                assert_eq!(primaries, 1, "{}", m.key);
                assert!(m.primary_param.is_some() && !m.always_on);
            } else {
                assert_eq!((primaries, m.always_on), (0, true), "limiter: always on, advanced only");
            }
            assert!(!m.description.is_empty());
        }
        let desmack = cat.iter().find(|m| m.key == "desmack").unwrap();
        assert_eq!(desmack.primary_param.as_deref(), Some("range"));
        let mains = &cat[0].params[0];
        assert_eq!(mains.choices.iter().map(|c| c.label.as_str()).collect::<Vec<_>>(), ["Auto", "50 Hz", "60 Hz"]);
    }

    #[test]
    fn live_settings_match_the_settings_file_shape() {
        let mut st = ModuleState::new(user_module_keys());
        st.set_param("desmack", "below_speech", 18.0);
        st.set_enabled("dehum", false);
        let live = st.live_settings();
        assert!(!live.is_enabled("dehum") && live.is_enabled("desmack"));
        assert_eq!(live.param("desmack", "below_speech"), Some(18.0));
        // A sidecar's `settings` reads back as the app's settings file and vice versa.
        let json = serde_json::to_string(&live).unwrap();
        assert_eq!(settings_from_json(json).unwrap(), st.settings());
        let side = serde_json::json!({ "settings": serde_json::from_str::<serde_json::Value>(&settings_to_json(st.settings())).unwrap() });
        assert_eq!(hearo_engine::snapshots::LiveSettings::from_sidecar(&side), Some(live));
    }

    #[test]
    fn json_roundtrip_and_tolerance() {
        let mut st = ModuleState::new(user_module_keys());
        st.set_param("desmack", "range", 12.0);
        st.set_enabled("deess", false);
        let s = st.settings();
        let back = settings_from_json(settings_to_json(s.clone())).unwrap();
        assert_eq!(back, s);

        // Older/newer files: unknown module, unknown param and unknown fields are ignored;
        // missing modules/params/fields are defaulted; values are clamped and snapped.
        let json = r#"{"modules":[
            {"key":"desmack","params":[{"key":"range","value":999},{"key":"future","value":1}],"future":true},
            {"key":"dehum","enabled":false,"params":[{"key":"mains","value":49},{"key":"harmonics","value":3.4}]},
            {"key":"teleporter","enabled":true}
        ],"version":7}"#;
        let s = settings_from_json(json.into()).unwrap();
        assert_eq!(s.modules.len(), 6, "every user module present");
        assert_eq!(get(&s, "desmack", "range"), 48.0, "clamped");
        assert_eq!(get(&s, "desmack", "threshold"), 14.0, "missing param → default");
        assert_eq!(get(&s, "dehum", "mains"), 50.0, "snapped to a choice");
        assert_eq!(get(&s, "dehum", "harmonics"), 3.0, "snapped to step");
        assert!(s.modules.iter().find(|m| m.key == "desmack").unwrap().enabled, "missing enabled → on");
        assert!(!s.modules.iter().find(|m| m.key == "dehum").unwrap().enabled);
        assert_eq!(s.modules.iter().find(|m| m.key == "deess").unwrap(), &default_settings().modules[4]);
        assert!(settings_from_json("{}".into()).unwrap() == default_settings());
        assert!(settings_from_json("not json".into()).is_err());
        assert!(settings_from_json(r#"{"modules":7}"#.into()).is_err());
    }

    #[test]
    fn limiter_stays_on_and_nan_means_default() {
        let mut st = ModuleState::new(user_module_keys());
        assert_eq!(st.set_enabled("limiter", false), Some((5, true)));
        assert_eq!(st.set_param("deess", "range", f32::NAN), Some((4, 1, 12.0)));
        assert_eq!(st.set_param("deess", "nope", 1.0), None);
        assert_eq!(st.set_param("nope", "range", 1.0), None);
    }

    #[test]
    fn settings_survive_a_rebuild() {
        // What the output follower does: a new engine (fresh Controls, fresh modules) gets the
        // remembered state pushed before audio flows.
        let mut st = ModuleState::new(user_module_keys());
        st.apply(&EngineSettings {
            modules: vec![ModuleSetting {
                key: "desmack".into(),
                enabled: false,
                params: vec![ParamValue { key: "range".into(), value: 9.0 }],
            }],
        });
        let (enabled, params) = st.for_supervisor();
        let slot = st.slot("desmack").unwrap();
        assert!(!enabled[slot]);
        assert_eq!(params[&(slot, 2)], 9.0);
        assert_eq!(params[&(st.slot("deess").unwrap(), 1)], 12.0, "defaults are pushed too");
        // And the chain built from those keys takes them over unchanged.
        let mut m = modules::create("desmack").unwrap();
        m.set_param(2, params[&(slot, 2)]);
        assert_eq!(m.param(2), 9.0);
    }
}
