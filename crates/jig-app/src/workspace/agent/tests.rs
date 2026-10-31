use std::time::{Duration, SystemTime};

#[test]
fn says_roughly_how_long_ago() {
    let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000);
    let ago = |seconds| super::ago(now, now - Duration::from_secs(seconds));
    assert_eq!(ago(20), "just now");
    assert_eq!(ago(300), "5 min ago");
    assert_eq!(ago(7_200), "2 h ago");
    assert_eq!(ago(100_000), "yesterday");
    assert_eq!(ago(5 * 86_400), "5 days ago");
}
