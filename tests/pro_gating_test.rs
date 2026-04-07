use night_amplifier::push_to::{PushToError, PUSH_TO_PLUGIN};
use night_amplifier::stacking::{StackingType, COMET_PLUGIN};

#[tokio::test]
async fn test_comet_plugin_gating() {
    // In Community repo, COMET_PLUGIN should be empty by default
    assert!(COMET_PLUGIN.get().is_none());

    // Check if StackingType::Comet is still reported but with appropriate info
    let info = StackingType::Comet.info();
    assert_eq!(info.id, StackingType::Comet);
}

#[test]
fn test_stacking_type_info_comet() {
    let info = StackingType::Comet.info();
    assert_eq!(info.id, StackingType::Comet);
    assert!(info.supports_stacking);
}

#[test]
fn test_push_to_plugin_not_registered_in_community() {
    // In the Community version, no Push-To plugin should be registered
    assert!(
        PUSH_TO_PLUGIN.get().is_none(),
        "PUSH_TO_PLUGIN must not be registered in the Community version"
    );
}

#[test]
fn test_push_to_plugin_required_error_exists() {
    // The PluginRequired variant must exist for the server to return proper gating responses
    let err = PushToError::PluginRequired;
    let msg = err.to_string();
    assert!(
        msg.contains("Pro"),
        "PluginRequired error should mention Pro: got '{}'",
        msg
    );
}
