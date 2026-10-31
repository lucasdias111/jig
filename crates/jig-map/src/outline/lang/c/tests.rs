use crate::outline::Lang;
use crate::outline::syntax::functions;

#[test]
fn lists_c_functions() {
    let source = "static int count;\nint save(struct store *s, int item) { return item; }\nstatic void free_it(void) {}\nchar *name_of(int id) { return 0; }\nint prototype(void);\n";
    let names: Vec<_> = functions(source, Lang::C)
        .into_iter()
        .map(|f| f.name)
        .collect();
    assert_eq!(names, ["save", "free_it", "name_of"]);
}

#[test]
fn lists_cpp_methods_in_classes_and_namespaces() {
    let source = "namespace tools {\nclass Store {\n public:\n  int save(int item) { return item; }\n  void later();\n};\nvoid Store::later() {}\ntemplate <typename T> T pick(T a) { return a; }\n}\nstruct Point { int len() { return 0; } };\nint main() { return 0; }\n";
    let found = functions(source, Lang::Cpp);
    let names: Vec<_> = found.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "tools::Store::save",
            "tools::Store::later",
            "tools::pick",
            "Point::len",
            "main"
        ]
    );
    assert_eq!(
        found[0].section.as_deref(),
        Some("namespace tools › class Store")
    );
    assert_eq!(found[1].section.as_deref(), Some("namespace tools › Store"));
    assert_eq!(found[1].short, "later");
    assert_eq!(found[3].section.as_deref(), Some("struct Point"));
}
