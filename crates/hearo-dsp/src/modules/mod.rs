//! Processing modules and the registry that builds them by key.

pub mod declick;
pub mod deess;
pub mod dehum;
pub mod deplosive;
pub mod desmack;
pub mod limiter;
pub mod utility;

use crate::Module;

/// Module keys the registry knows, in default chain order.
pub const KEYS: &[&str] = &["dehum", "deplosive", "declick", "desmack", "deess", "limiter", "gain"];

/// Build a module from its CLI/settings key.
pub fn create(key: &str) -> Option<Box<dyn Module>> {
    match key {
        "dehum" => Some(Box::new(dehum::DeHum::new())),
        "deplosive" => Some(Box::new(deplosive::DePlosive::new())),
        "declick" => Some(Box::new(declick::DeClick::new())),
        "desmack" => Some(Box::new(desmack::DeSmack::new())),
        "deess" => Some(Box::new(deess::DeEss::new())),
        "limiter" => Some(Box::new(limiter::Limiter::new())),
        "gain" => Some(Box::new(utility::Gain::new(0.0))),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn every_key_builds() {
        for k in super::KEYS {
            assert!(super::create(k).is_some(), "{k}");
        }
        assert!(super::create("nope").is_none());
    }

    /// The module catalog the UI is built from: every cleanup module has a primary ("how much")
    /// parameter, and every parameter description is self-consistent.
    #[test]
    fn param_metadata_is_consistent() {
        use crate::ModuleId;
        for k in super::KEYS {
            let mut m = super::create(k).unwrap();
            m.prepare(48_000.0, 64, 2);
            let params = m.params().to_vec();
            assert!(params.len() <= 8, "{k}: more params than the engine's control slots");
            if ModuleId::CLEANUP.contains(&m.id()) {
                let p = m.primary_param().unwrap_or_else(|| panic!("{k} has no primary param"));
                assert!(p < params.len(), "{k}");
            }
            if m.id() == ModuleId::Limiter {
                assert_eq!(m.primary_param(), None, "the limiter's ceiling is advanced only");
            }
            for (i, d) in params.iter().enumerate() {
                assert!(d.min <= d.default && d.default <= d.max, "{k}.{}", d.key);
                assert_eq!(d.snap(d.default), d.default, "{k}.{}: default is not a legal value", d.key);
                assert!(d.choices.iter().all(|c| (d.min..=d.max).contains(&c.value)), "{k}.{}", d.key);
                assert_eq!(m.param(i), d.default, "{k}.{}: module starts at its default", d.key);
                // Every legal value round-trips through the module exactly.
                for v in [d.min, d.max, 0.5 * (d.min + d.max), 25.0, 55.0] {
                    m.set_param(i, v);
                    assert_eq!(m.param(i), d.snap(v), "{k}.{}: module and snap disagree at {v}", d.key);
                }
            }
        }
    }
}
