use crate::map::outline::Lang;
use crate::map::outline::syntax::functions;

const PY: Lang = Lang::Python;

#[test]
fn lists_functions_and_methods() {
    let source = "def free():\n    pass\n\n@cached\ndef decorated(a):\n    return a\n\nclass Store:\n    def save(self, item):\n        pass\n\n    class Inner:\n        def deep(self):\n            pass\n\nasync def later():\n    await go()\n";
    let found = functions(source, PY);
    let names: Vec<_> = found.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "free",
            "decorated",
            "Store.save",
            "Store.Inner.deep",
            "later"
        ]
    );
    assert_eq!(found[2].section.as_deref(), Some("class Store"));
    assert_eq!(
        found[3].section.as_deref(),
        Some("class Store › class Inner")
    );
    assert_eq!(found[2].short, "save");
}
