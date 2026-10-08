//! The plugin set: licence gating, slots, and how installs merge.

use super::*;
use crate::error::Result;
use crate::frame::Frame;
use crate::render::stretch::SaturationBoostConfig;

struct FakeSaturation(&'static str);

impl SaturationPlugin for FakeSaturation {
    fn apply_boost(&self, _: &mut Frame, _: &SaturationBoostConfig) -> Result<()> {
        Ok(())
    }

    fn apply_boost_slice(&self, row: &mut [f32], _: &SaturationBoostConfig) {
        row.fill(self.0.len() as f32);
    }
}

fn name_of(plugins: &Plugins) -> f32 {
    let mut row = [0.0; 3];
    let plugin = plugins.saturation().expect("a saturation plugin");
    plugin.apply_boost_slice(&mut row, &SaturationBoostConfig::default());
    row[0]
}

#[test]
fn community_has_nothing_in_any_slot() {
    let plugins = Plugins::none().always_licensed();
    assert!(plugins.rejection().is_none());
    assert!(plugins.saturation().is_none());
    assert!(plugins.push_to_solver().is_none());
    assert!(plugins.registered().is_empty());
    assert!(!plugins.ships_rejection());
}

/// Nothing in this test binary activates the process licence, so a set that follows it
/// answers nothing — while still saying what it ships.
#[test]
fn an_unlicensed_set_answers_nothing_but_still_ships() {
    let plugins = Plugins::none().with_saturation(Arc::new(FakeSaturation("a")));
    assert!(!crate::license::is_pro_active());
    assert!(plugins.saturation().is_none());
    assert_eq!(plugins.registered(), ["saturation"]);

    assert!(plugins.always_licensed().saturation().is_some());
}

#[test]
fn a_filled_slot_answers_and_is_named() {
    let plugins = Plugins::none()
        .with_saturation(Arc::new(FakeSaturation("abc")))
        .always_licensed();
    assert_eq!(name_of(&plugins), 3.0);
    assert_eq!(plugins.registered(), ["saturation"]);
}

/// A later install only fills what the earlier one left empty — Pro's tests register
/// a set, then a wider one, and the first plugin of each slot stays.
#[test]
fn a_merge_keeps_the_first_plugin_of_each_slot() {
    let first = Plugins::none().with_saturation(Arc::new(FakeSaturation("one")));
    let second = Plugins::none()
        .with_saturation(Arc::new(FakeSaturation("three")))
        .always_licensed();

    let merged = first.or(second);
    assert_eq!(name_of(&merged), 3.0, "the first set's plugin");
    assert!(merged.saturation().is_some(), "licence-free if either set was");
}

/// Cloning shares the set rather than copying the slots.
#[test]
fn a_clone_is_the_same_set() {
    let plugins = Plugins::none().with_saturation(Arc::new(FakeSaturation("ab")));
    let clone = plugins.clone();
    assert!(Arc::ptr_eq(&plugins.0, &clone.0));
}
