use crate::domain::outline::Lang;
use crate::domain::outline::syntax::functions;

const ZIG: Lang = Lang::Zig;

#[test]
fn lists_functions_and_those_in_containers() {
    let source = "const std = @import(\"std\");\npub fn save(item: i32) i32 { return item; }\nconst Store = struct {\n    fn flush(self: *Store) void {}\n};\nconst Color = enum { red, fn paint() void {} };\n";
    let found = functions(source, ZIG);
    let names: Vec<_> = found.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(names, ["save", "Store.flush", "Color.paint"]);
    assert_eq!(found[1].section.as_deref(), Some("struct Store"));
    assert_eq!(found[2].section.as_deref(), Some("enum Color"));
}
