use crate::outline::Lang;
use crate::outline::syntax::functions;

const GO: Lang = Lang::Go;

#[test]
fn lists_functions_and_methods_by_receiver() {
    let source = "package main\n\nfunc free() {}\n\nfunc (s *Store) Save(item int) {}\n\nfunc (l List[T]) Len() int { return 0 }\n";
    let found = functions(source, GO);
    let names: Vec<_> = found.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["free", "Store.Save", "List.Len"]);
    assert_eq!(found[1].section.as_deref(), Some("Store"));
    assert_eq!(found[1].short, "Save");
}
