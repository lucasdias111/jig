use crate::domain::outline::Lang;
use crate::domain::outline::syntax::functions;

const JAVA: Lang = Lang::Java;

#[test]
fn lists_methods_and_constructors_in_their_types() {
    let source = "class Store {\n  Store() {}\n  int save(int item) { return item; }\n  abstract void later();\n  static class Inner { void deep() {} }\n}\ninterface Named { default String name() { return \"\"; } }\nenum Color { RED; void paint() {} }\nrecord Point(int x) { int twice() { return x * 2; } }\n";
    let found = functions(source, JAVA);
    let names: Vec<_> = found.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Store.Store",
            "Store.save",
            "Store.Inner.deep",
            "Named.name",
            "Color.paint",
            "Point.twice"
        ]
    );
    assert_eq!(
        found[2].section.as_deref(),
        Some("class Store › class Inner")
    );
    assert_eq!(found[3].section.as_deref(), Some("interface Named"));
    assert_eq!(found[4].section.as_deref(), Some("enum Color"));
}
