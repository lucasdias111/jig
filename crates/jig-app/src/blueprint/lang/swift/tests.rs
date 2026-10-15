use crate::blueprint::lang::testing::{after, flow, flow_of, node, runs_forward, titles, wires};
use crate::blueprint::{Kind, Lang, callers_in, functions};

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

#[test]
fn statements_run_in_order_from_the_entry() {
    let graph = flow(
        "func save(item: Int, _ names: String...) throws -> Int {\n    let x = compute(item)\n    var y: Int = 2\n    self.items.append(x)\n    y += 1\n    try risky()\n    defer { done() }\n    return x\n}\n",
        SWIFT,
    );
    assert_eq!(
        titles(&graph),
        [
            "save",
            "Let x",
            "Var y",
            "Call append",
            "Set y",
            "Call risky",
            "Defer",
            "Return"
        ]
    );
    assert_eq!(
        graph.nodes[0].code,
        ["item: Int", "_ names: String...", "→ Int"]
    );
    assert_eq!(graph.nodes[1].kind, Kind::Let);
    assert!(graph.nodes[1].callee.is_some());
}

#[test]
fn if_chains_else_if_and_guard_leaves_from_false() {
    let graph = flow(
        "func go(x: Int, maybe: Int?) -> Int {\n    if x > 2 { big(x) } else if x < 0 { neg() } else { small() }\n    guard let z = maybe else { return 0 }\n    done(z)\n    return z\n}\n",
        SWIFT,
    );
    assert_eq!(graph.nodes[node(&graph, "Branch")].code, ["x > 2"]);
    for from in ["Call big", "Call neg", "Call small"] {
        assert_eq!(after(&graph, from, 0), ["Branch"]);
    }
    let guard = graph
        .nodes
        .iter()
        .rposition(|node| node.title == "Branch")
        .unwrap();
    assert_eq!(graph.nodes[guard].code, ["let z = maybe"]);
    assert_eq!(
        super::super::testing::next(&graph, guard, 0),
        [node(&graph, "Call done")]
    );
    assert_eq!(
        graph.nodes[super::super::testing::next(&graph, guard, 1)[0]].title,
        "Return"
    );
    assert!(wires(&graph).contains(&("go", "maybe", "Branch")));
}

#[test]
fn loops_switch_and_do_catch() {
    let graph = flow(
        "func go(x: Int, items: [Int]) {\n    for v in items { if v > 2 { break }; continue }\n    while busy() { wait() }\n    repeat { spin() } while again\n    switch x {\n    case 1, 2:\n        one()\n        fallthrough\n    case 3:\n        three()\n    default:\n        other()\n    }\n    do { try risky() } catch let error as MyError { report(error) } catch { log(error) }\n    done()\n}\n",
        SWIFT,
    );
    assert_eq!(graph.nodes[node(&graph, "For Each")].code, ["v in items"]);
    assert_eq!(after(&graph, "Break", 0), ["While"]);
    assert_eq!(after(&graph, "Repeat While", 1), ["Switch"]);
    let switch = node(&graph, "Switch");
    assert_eq!(graph.nodes[switch].outs, ["1, 2", "3", "default"]);
    assert_eq!(after(&graph, "Call one", 0), ["Call three"]);
    assert_eq!(after(&graph, "Call three", 0), ["Try"]);
    let attempt = node(&graph, "Try");
    assert_eq!(
        graph.nodes[attempt].outs,
        ["Body", "catch let error as MyEr…", "catch"]
    );
    for from in ["Call risky", "Call report", "Call log"] {
        assert_eq!(after(&graph, from, 0), ["Call done"]);
    }
    assert!(runs_forward(&graph));
}

#[test]
fn fatal_error_ends_a_path() {
    let graph = flow(
        "func go(ok: Bool) {\n    if !ok { fatalError(\"no\") }\n    done()\n}\n",
        SWIFT,
    );
    assert_eq!(graph.nodes[node(&graph, "Panic")].kind, Kind::Exit);
    assert_eq!(after(&graph, "Branch", 1), ["Call done"]);
}

#[test]
fn reads_wire_back_to_declarations_and_parameters() {
    let graph = flow(
        "func go(a: Int, label b: String) {\n    let x = a + 1\n    log(x, b, self.a)\n    let f = { (q: Int) in q + x }\n    for v in items { use(v, a) }\n    save(f)\n}\n",
        SWIFT,
    );
    let wires = wires(&graph);
    assert!(wires.contains(&("go", "a", "Let x")));
    assert!(wires.contains(&("Let x", "x", "Call log")));
    assert!(wires.contains(&("go", "b", "Call log")));
    assert!(!wires.contains(&("go", "a", "Call log")), "a member, not a");
    assert!(wires.contains(&("Let x", "x", "Let f")));
    assert!(wires.contains(&("go", "a", "Call use")));
    assert!(wires.contains(&("Let f", "f", "Call save")));
    assert!(
        !wires
            .iter()
            .any(|(_, name, _)| *name == "label" || *name == "q")
    );
}

#[test]
fn finds_callers_in_the_file() {
    let source = "func save() {}\nclass Store {\n    func flush() {\n        save()\n        self.save()\n    }\n}\nfunc other() { saveAll() }\n";
    let found = functions(source, SWIFT);
    let callers: Vec<_> = callers_in(source, SWIFT, &found[0])
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(callers, ["Store.flush", "Store.flush"]);
}

#[test]
fn an_inits_flow() {
    let graph = flow_of(
        "class Store {\n    init(items: [Int]) {\n        self.items = items\n        load()\n    }\n}\n",
        SWIFT,
        "Store.init",
    );
    assert_eq!(
        titles(&graph),
        ["Store.init", "Set self.items", "Call load"]
    );
}
