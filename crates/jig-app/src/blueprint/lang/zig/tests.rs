use crate::blueprint::lang::testing::{after, flow, flow_of, node, runs_forward, titles, wires};
use crate::blueprint::{Kind, Lang, callers_in, functions};

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

#[test]
fn statements_run_in_order_from_the_entry() {
    let graph = flow(
        "pub fn save(s: *Store, item: i32) !i32 {\n    const x = compute(item);\n    var y: i32 = 2;\n    s.items[0] = x;\n    y += 1;\n    try go();\n    defer done();\n    s.flush();\n    return x;\n}\n",
        ZIG,
    );
    assert_eq!(
        titles(&graph),
        [
            "save",
            "Const x",
            "Var y",
            "Set s.items[0]",
            "Set y",
            "Call go",
            "Defer done",
            "Call flush",
            "Return"
        ]
    );
    assert_eq!(graph.nodes[0].code, ["s: *Store", "item: i32", "→ !i32"]);
    assert_eq!(graph.nodes[1].kind, Kind::Let);
    assert!(graph.nodes[1].callee.is_some());
}

#[test]
fn branches_loops_and_a_switch() {
    let graph = flow(
        "fn go(x: i32, items: []i32) void {\n    if (x > 2) {\n        big(x);\n    } else if (x < 0) {\n        neg();\n    } else {\n        small();\n    }\n    for (items) |v| {\n        if (v > 9) break;\n        continue;\n    }\n    while (busy()) {\n        wait();\n    }\n    switch (x) {\n        1, 2 => one(),\n        else => other(),\n    }\n    done();\n}\n",
        ZIG,
    );
    assert_eq!(graph.nodes[node(&graph, "Branch")].code, ["x > 2"]);
    for from in ["Call big", "Call neg", "Call small"] {
        assert_eq!(after(&graph, from, 0), ["For Each"]);
    }
    assert_eq!(graph.nodes[node(&graph, "For Each")].code, ["items |v|"]);
    assert_eq!(after(&graph, "Break", 0), ["While"]);
    assert_eq!(graph.nodes[node(&graph, "Continue")].kind, Kind::Exit);
    let switch = node(&graph, "Switch");
    assert_eq!(graph.nodes[switch].outs, ["1, 2", "else"]);
    assert_eq!(after(&graph, "Call one", 0), ["Call done"]);
    assert_eq!(after(&graph, "Call other", 0), ["Call done"]);
    assert!(runs_forward(&graph));
}

#[test]
fn a_panic_ends_a_path() {
    let graph = flow(
        "fn go(ok: bool) void {\n    if (!ok) {\n        @panic(\"no\");\n    }\n    done();\n}\n",
        ZIG,
    );
    assert_eq!(graph.nodes[node(&graph, "Panic")].kind, Kind::Exit);
    assert_eq!(after(&graph, "Branch", 1), ["Call done"]);
}

#[test]
fn reads_wire_back_to_declarations_parameters_and_not_payloads() {
    let graph = flow(
        "fn go(s: *Store, a: i32) void {\n    const x = a + 1;\n    log(x, s.a);\n    for (s.items) |a| {\n        use(a);\n    }\n    save(x, a);\n}\n",
        ZIG,
    );
    let wires = wires(&graph);
    assert!(wires.contains(&("go", "a", "Const x")));
    assert!(wires.contains(&("Const x", "x", "Call log")));
    assert!(wires.contains(&("go", "s", "Call log")));
    assert!(!wires.contains(&("go", "a", "Call log")), "a field, not a");
    assert!(
        !wires.contains(&("go", "a", "Call use")),
        "the loop's own a"
    );
    assert!(wires.contains(&("go", "a", "Call save")));
}

#[test]
fn finds_callers_in_the_file() {
    let source = "fn save() void {}\nconst Store = struct {\n    fn flush(self: *Store) void {\n        save();\n        self.save();\n    }\n};\nfn other() void { saveAll(); }\n";
    let found = functions(source, ZIG);
    let callers: Vec<_> = callers_in(source, ZIG, &found[0])
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(callers, ["Store.flush", "Store.flush"]);
}

#[test]
fn a_methods_flow() {
    let graph = flow_of(
        "const Store = struct {\n    fn flush(self: *Store) void {\n        self.items.clear();\n    }\n};\n",
        ZIG,
        "Store.flush",
    );
    assert_eq!(titles(&graph), ["Store.flush", "Call clear"]);
}
