use super::*;

#[test]
fn test_qhy_provider_init() {
    let provider = QhyProvider::new();
    assert_eq!(provider.name(), "QHY");
}
