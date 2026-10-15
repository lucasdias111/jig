use crate::blueprint::lang::testing::{after, flow, flow_of, node, runs_forward, titles, wires};
use crate::blueprint::{Kind, Lang, callers_in, functions};

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

#[test]
fn statements_run_in_order_from_the_entry() {
    let graph = flow(
        "class A {\n  int save(int item, String... names) {\n    int x = compute(item), y = 2;\n    this.items.add(x);\n    count = 3;\n    x++;\n    new Thing(x);\n    super.close();\n    return x;\n  }\n}\n",
        JAVA,
    );
    assert_eq!(
        titles(&graph),
        [
            "A.save",
            "Let x, y",
            "Call add",
            "Set count",
            "Set x",
            "New Thing",
            "Call close",
            "Return"
        ]
    );
    assert_eq!(
        graph.nodes[0].code,
        ["int item", "String... names", "→ int"]
    );
    assert_eq!(graph.nodes[1].kind, Kind::Let);
    assert!(graph.nodes[1].callee.is_some());
    assert!(graph.nodes[5].callee.is_some(), "a new knows its class");
}

#[test]
fn if_chains_else_if() {
    let graph = flow(
        "class A {\n  void go(int x) {\n    if (x > 2) { big(); } else if (x < 0) neg(); else { small(); }\n    done();\n  }\n}\n",
        JAVA,
    );
    assert_eq!(
        titles(&graph),
        [
            "A.go",
            "Branch",
            "Call big",
            "Branch",
            "Call neg",
            "Call small",
            "Call done"
        ]
    );
    assert_eq!(graph.nodes[1].code, ["x > 2"]);
    for from in ["Call big", "Call neg", "Call small"] {
        assert_eq!(after(&graph, from, 0), ["Call done"]);
    }
}

#[test]
fn loops() {
    let graph = flow(
        "class A {\n  void go(String[] names) {\n    for (int i = 0; i < 3; i++) tick(i);\n    for (String v : names) { if (v == null) break; continue; }\n    while (busy()) wait();\n    do { spin(); } while (again);\n    done();\n  }\n}\n",
        JAVA,
    );
    assert_eq!(
        graph.nodes[node(&graph, "For")].code,
        ["int i = 0; i < 3; i++"]
    );
    assert_eq!(
        graph.nodes[node(&graph, "For Each")].code,
        ["String v : names"]
    );
    assert_eq!(after(&graph, "Break", 0), ["While"]);
    assert_eq!(after(&graph, "Do While", 1), ["Call done"]);
}

#[test]
fn case_groups_fall_through_and_rules_dont() {
    let graph = flow(
        "class A {\n  void go(int x) {\n    switch (x) {\n      case 1:\n      case 2: one();\n      case 3: three(); break;\n      default: other();\n    }\n    switch (x) {\n      case 1 -> first();\n      case 2, 3 -> second();\n    }\n    done();\n  }\n}\n",
        JAVA,
    );
    let switch = node(&graph, "Switch");
    assert_eq!(graph.nodes[switch].outs, ["1", "2", "3", "default"]);
    assert_eq!(
        after(&graph, "Switch", 0),
        ["Call one"],
        "an empty case runs on"
    );
    assert_eq!(after(&graph, "Call one", 0), ["Call three"]);
    assert_eq!(after(&graph, "Break", 0), ["Switch"]);
    let rules = graph
        .nodes
        .iter()
        .rposition(|node| node.title == "Switch")
        .unwrap();
    assert_eq!(graph.nodes[rules].outs, ["1", "2, 3", "Default"]);
    assert_eq!(after(&graph, "Call first", 0), ["Call done"]);
    assert_eq!(after(&graph, "Call second", 0), ["Call done"]);
}

#[test]
fn try_has_an_output_per_catch_then_finally() {
    let graph = flow(
        "class A {\n  void go() {\n    try { risky(); } catch (IOException | RuntimeException e) { report(e); } catch (Exception e) { log(e); } finally { clean(); }\n    throw new Error();\n  }\n}\n",
        JAVA,
    );
    let attempt = node(&graph, "Try");
    assert_eq!(
        graph.nodes[attempt].outs,
        ["Body", "catch IOException | Run…", "catch Exception"]
    );
    for from in ["Call risky", "Call report", "Call log"] {
        assert_eq!(after(&graph, from, 0), ["Call clean"]);
    }
    assert_eq!(after(&graph, "Call clean", 0), ["Throw"]);
    assert!(runs_forward(&graph));
}

#[test]
fn reads_wire_back_to_declarations_and_parameters() {
    let graph = flow(
        "class A {\n  void go(int a, String b) {\n    int x = a + 1;\n    log(x, b, this.name, name());\n    Runnable r = () -> use(x);\n    for (String v : items) { use(v, a); }\n    try { r.run(); } catch (Exception a) { log(a); }\n  }\n}\n",
        JAVA,
    );
    let wires = wires(&graph);
    assert!(wires.contains(&("A.go", "a", "Let x")));
    assert!(wires.contains(&("Let x", "x", "Call log")));
    assert!(wires.contains(&("A.go", "b", "Call log")));
    assert!(wires.contains(&("Let x", "x", "Let r")));
    assert!(wires.contains(&("A.go", "a", "Call use")));
    assert!(wires.contains(&("Let r", "r", "Call run")));
    assert!(
        !wires.contains(&("A.go", "a", "Call log")),
        "the catch's own a"
    );
    assert!(!wires.iter().any(|(_, name, _)| *name == "name"));
}

#[test]
fn finds_callers_in_the_file() {
    let source = "class A {\n  void save() {}\n  void flush() { save(); this.save(); }\n  void other() { saveAll(); }\n}\n";
    let found = functions(source, JAVA);
    let callers: Vec<_> = callers_in(source, JAVA, &found[0])
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(callers, ["A.flush", "A.flush"]);
}

#[test]
fn a_constructors_flow() {
    let graph = flow_of(
        "class A {\n  A(int x) {\n    super(x);\n    this.x = x;\n  }\n}\n",
        JAVA,
        "A.A",
    );
    assert_eq!(titles(&graph), ["A.A", "Call super", "Set this.x"]);
}
