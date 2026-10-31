use super::group_digits;

#[test]
fn digits_are_grouped() {
    assert_eq!(group_digits(7), "7");
    assert_eq!(group_digits(1204), "1,204");
    assert_eq!(group_digits(200000), "200,000");
}
