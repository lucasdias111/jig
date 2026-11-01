use crate::domain::outline::Lang;
use crate::domain::outline::syntax::{containing, functions};

#[test]
fn lists_functions_arrows_methods_and_namespaces() {
    let source = "function free() {}\n\
                  export function exported() {}\n\
                  const arrow = (a: number) => a + 1;\n\
                  export const handler = async () => { await go(); };\n\
                  const notAFunction = 3;\n\
                  class Store {\n  constructor() {}\n  save(item: Item): void { this.items.push(item); }\n  onClick = () => {};\n  abstract?: number;\n}\n\
                  namespace Tools { export function help() {} }\n";
    let found = functions(source, Lang::TypeScript);
    let names: Vec<_> = found.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "free",
            "exported",
            "arrow",
            "handler",
            "Store.constructor",
            "Store.save",
            "Store.onClick",
            "Tools.help"
        ]
    );
    let sections: Vec<_> = found.iter().map(|f| f.section.as_deref()).collect();
    assert_eq!(sections[5], Some("class Store"));
    assert_eq!(sections[7], Some("namespace Tools"));
    assert_eq!(found[5].short, "save");
    assert_eq!(&source[found[5].name_at..found[5].name_at + 4], "save");
    // A `const` arrow is the whole statement.
    assert!(source[found[2].range.clone()].starts_with("const arrow"));
    assert_eq!(containing(&found, source.find("go()").unwrap()), Some(3));
}
