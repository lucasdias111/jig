use crate::blueprint::lang::testing::{after, flow, flow_of, node, runs_forward, titles, wires};
use crate::blueprint::{Kind, Lang, callers_in, functions};

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

#[test]
fn statements_run_in_order_from_the_entry() {
    let graph = flow(
        "int save(struct store *s, int item) {\n    int x = compute(item), *p = &x;\n    s->items[0] = x;\n    x += 1;\n    x++;\n    log_it(s->name);\n    return x;\n}\n",
        Lang::C,
    );
    assert_eq!(
        titles(&graph),
        [
            "save",
            "Let x, p",
            "Set s->items[0]",
            "Set x",
            "Set x",
            "Call log_it",
            "Return"
        ]
    );
    assert_eq!(
        graph.nodes[0].code,
        ["struct store *s", "int item", "→ int"]
    );
    assert_eq!(graph.nodes[1].kind, Kind::Let);
    assert!(graph.nodes[1].callee.is_some());
}

#[test]
fn branches_loops_and_a_falling_switch() {
    let graph = flow(
        "void go(int x) {\n    if (x > 2) { big(x); } else if (x < 0) neg(); else { small(); }\n    for (int i = 0; i < 3; i++) tick(i);\n    while (busy()) { if (stop) break; wait(); }\n    do { spin(); } while (again);\n    switch (x) {\n    case 1:\n    case 2: one();\n    case 3: three(); break;\n    default: other();\n    }\n    done();\n}\n",
        Lang::C,
    );
    assert_eq!(graph.nodes[node(&graph, "Branch")].code, ["x > 2"]);
    assert_eq!(after(&graph, "Call big", 0), ["For"]);
    assert_eq!(
        graph.nodes[node(&graph, "For")].code,
        ["int i = 0; i < 3; i++"]
    );
    assert_eq!(after(&graph, "Break", 0), ["Do While"]);
    let switch = node(&graph, "Switch");
    assert_eq!(graph.nodes[switch].outs, ["1", "2", "3", "default"]);
    assert_eq!(after(&graph, "Switch", 0), ["Call one"]);
    assert_eq!(after(&graph, "Call one", 0), ["Call three"]);
    assert_eq!(after(&graph, "Call other", 0), ["Call done"]);
    assert!(runs_forward(&graph));
}

#[test]
fn goto_and_abort_end_a_path() {
    let graph = flow(
        "void go(int ok) {\n    if (!ok) abort();\n    goto out;\nout:\n    done();\n}\n",
        Lang::C,
    );
    assert_eq!(graph.nodes[node(&graph, "Panic")].kind, Kind::Exit);
    assert_eq!(graph.nodes[node(&graph, "Goto")].kind, Kind::Exit);
    assert!(titles(&graph).contains(&"Call done"));
}

#[test]
fn cpp_range_for_try_and_new() {
    let graph = flow_of(
        "class Store {\n  int save(int item) {\n    auto x = compute(item);\n    for (auto& v : items) { if (!v) break; }\n    try { risky(); } catch (const std::exception& e) { report(e); } catch (...) {}\n    auto p = new Thing(x);\n    this->items.push_back(x);\n    std::sort(a, b);\n    throw std::runtime_error(\"no\");\n  }\n};\n",
        Lang::Cpp,
        "Store::save",
    );
    assert_eq!(
        graph.nodes[node(&graph, "For Each")].code,
        ["auto& v : items"]
    );
    let attempt = node(&graph, "Try");
    assert_eq!(
        graph.nodes[attempt].outs,
        ["Body", "catch (const std::excep…", "catch (...)"]
    );
    assert_eq!(after(&graph, "Call risky", 0), ["Let p"]);
    assert_eq!(after(&graph, "Try", 2), ["Let p"]);
    assert!(titles(&graph).contains(&"Call push_back"));
    assert!(titles(&graph).contains(&"Call sort"));
    assert_eq!(graph.nodes[node(&graph, "Throw")].kind, Kind::Exit);
}

#[test]
fn reads_wire_back_to_declarations_and_parameters() {
    let graph = flow(
        "int go(struct store *s, int a) {\n    int x = a + 1;\n    int y;\n    y = x * 2;\n    log_it(x, y, s->a);\n    for (int i = 0; i < x; i++) use(i, a);\n    return y;\n}\n",
        Lang::C,
    );
    let wires = wires(&graph);
    assert!(wires.contains(&("go", "a", "Let x")));
    assert!(wires.contains(&("Let x", "x", "Set y")));
    assert!(wires.contains(&("Let y", "y", "Call log_it")));
    assert!(wires.contains(&("go", "s", "Call log_it")));
    assert!(
        !wires.contains(&("go", "a", "Call log_it")),
        "a field, not a"
    );
    assert!(wires.contains(&("Let x", "x", "For")));
    assert!(wires.contains(&("go", "a", "Call use")));
    assert!(wires.contains(&("Let y", "y", "Return")));
}

#[test]
fn cpp_lambdas_and_catches_hide_outer_names() {
    let graph = flow(
        "int go(int e, int q) {\n    auto f = [&](int q) { return q + e; };\n    try { f(1); } catch (const std::exception& e) { report(e); }\n    return q;\n}\n",
        Lang::Cpp,
    );
    let wires = wires(&graph);
    assert!(wires.contains(&("go", "e", "Let f")));
    assert!(!wires.contains(&("go", "q", "Let f")));
    assert!(!wires.contains(&("go", "e", "Call report")));
    assert!(wires.contains(&("go", "q", "Return")));
}

#[test]
fn finds_callers_in_the_file() {
    let source =
        "void save(void) {}\nvoid flush(void) { save(); }\nvoid other(void) { save_all(); }\n";
    let found = functions(source, Lang::C);
    let callers: Vec<_> = callers_in(source, Lang::C, &found[0])
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(callers, ["flush"]);
}
