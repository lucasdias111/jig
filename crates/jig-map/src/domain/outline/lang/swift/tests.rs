use crate::domain::outline::Lang;
use crate::domain::outline::syntax::functions;

const SWIFT: Lang = Lang::Swift;

#[test]
fn lists_functions_in_types_and_extensions() {
    let source = "func free() {}\nclass Store {\n    func save(item: Int) -> Int { return item }\n    init() {}\n}\nextension Store { func more() {} }\nstruct Point { func len() -> Int { return 0 } }\nprotocol Named { func name() -> String }\n";
    let found = functions(source, SWIFT);
    let names: Vec<_> = found.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "free",
            "Store.save",
            "Store.init",
            "Store.more",
            "Point.len"
        ]
    );
    assert_eq!(found[1].section.as_deref(), Some("class Store"));
    assert_eq!(found[3].section.as_deref(), Some("extension Store"));
    assert_eq!(found[4].section.as_deref(), Some("struct Point"));
}
