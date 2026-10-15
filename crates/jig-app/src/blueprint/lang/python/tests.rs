use crate::blueprint::lang::testing::{after, flow, flow_of, node, runs_forward, titles, wires};
use crate::blueprint::{Kind, Lang, callers_in, functions};

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

#[test]
fn statements_run_in_order_from_the_entry() {
    let graph = flow(
        "def go(a: int, *rest) -> int:\n    x = compute(a)\n    self.items.append(x)\n    self.count = 2\n    x += 1\n    await save()\n    return x\n",
        PY,
    );
    assert_eq!(
        titles(&graph),
        [
            "go",
            "Set x",
            "Call append",
            "Set self.count",
            "Set x",
            "Call save",
            "Return"
        ]
    );
    assert_eq!(graph.nodes[0].code, ["a: int", "*rest", "→ int"]);
    assert_eq!(graph.nodes[1].kind, Kind::Let, "a name's assignment binds");
    assert_eq!(graph.nodes[3].kind, Kind::Statement, "a field's doesn't");
    assert!(graph.nodes[1].callee.is_some());
    for ix in 0..6 {
        assert_eq!(super::super::testing::next(&graph, ix, 0), [ix + 1]);
    }
}

#[test]
fn elif_chains_from_false_and_else_ends_it() {
    let graph = flow(
        "def go(x):\n    if x > 2:\n        big()\n    elif x < 0:\n        neg()\n    else:\n        small()\n    done()\n",
        PY,
    );
    assert_eq!(
        titles(&graph),
        [
            "go",
            "Branch",
            "Call big",
            "Branch",
            "Call neg",
            "Call small",
            "Call done"
        ]
    );
    assert_eq!(graph.nodes[1].code, ["x > 2"]);
    assert_eq!(after(&graph, "Branch", 1), ["Branch"]);
    assert_eq!(graph.nodes[3].code, ["x < 0"]);
    for from in ["Call big", "Call neg", "Call small"] {
        assert_eq!(after(&graph, from, 0), ["Call done"]);
    }
}

#[test]
fn a_loops_else_runs_when_it_completes_not_when_it_breaks() {
    let graph = flow(
        "def go(items):\n    for i, v in enumerate(items):\n        if v:\n            break\n        continue\n    else:\n        nobreak()\n    while busy():\n        wait()\n    done()\n",
        PY,
    );
    let each = node(&graph, "For Each");
    assert_eq!(graph.nodes[each].code, ["i, v in enumerate(items)"]);
    assert_eq!(after(&graph, "For Each", 1), ["Call nobreak"]);
    assert_eq!(after(&graph, "Break", 0), ["While"]);
    assert_eq!(after(&graph, "Call nobreak", 0), ["While"]);
    assert_eq!(graph.nodes[node(&graph, "Continue")].kind, Kind::Exit);
    assert_eq!(after(&graph, "While", 1), ["Call done"]);
}

#[test]
fn try_has_an_output_per_except_then_else_and_finally() {
    let graph = flow(
        "def go():\n    try:\n        risky()\n    except ValueError as e:\n        report(e)\n    except:\n        pass\n    else:\n        fine()\n    finally:\n        clean()\n    raise Error()\n",
        PY,
    );
    let attempt = node(&graph, "Try");
    assert_eq!(
        graph.nodes[attempt].outs,
        ["Body", "except ValueError as e", "except"]
    );
    assert_eq!(after(&graph, "Call risky", 0), ["Call fine"]);
    assert_eq!(after(&graph, "Call fine", 0), ["Call clean"]);
    assert_eq!(after(&graph, "Call report", 0), ["Call clean"]);
    // `except: pass` goes straight on to `finally`.
    assert_eq!(after(&graph, "Try", 2), ["Call clean"]);
    assert_eq!(after(&graph, "Call clean", 0), ["Raise"]);
    assert_eq!(graph.nodes[node(&graph, "Raise")].kind, Kind::Exit);
}

#[test]
fn with_runs_its_body_and_match_has_an_output_per_case() {
    let graph = flow(
        "def go(p, x):\n    with open(p) as f:\n        read(f)\n    match x:\n        case 1 | 2:\n            one()\n        case _:\n            other()\n    done()\n",
        PY,
    );
    assert_eq!(graph.nodes[node(&graph, "With")].code, ["open(p) as f"]);
    assert_eq!(after(&graph, "With", 0), ["Call read"]);
    let matching = node(&graph, "Match");
    assert_eq!(graph.nodes[matching].outs, ["1 | 2", "_"]);
    assert_eq!(graph.nodes[matching].code, ["x"]);
    assert_eq!(after(&graph, "Call one", 0), ["Call done"]);
    assert_eq!(after(&graph, "Call other", 0), ["Call done"]);
}

#[test]
fn a_match_without_a_wildcard_goes_on() {
    let graph = flow(
        "def go(x):\n    match x:\n        case 1:\n            one()\n    done()\n",
        PY,
    );
    assert_eq!(graph.nodes[node(&graph, "Match")].outs, ["1", "Default"]);
    assert_eq!(after(&graph, "Match", 1), ["Call done"]);
}

#[test]
fn nested_definitions_and_pass_are_skipped() {
    let graph = flow(
        "def go():\n    # a note\n    def helper():\n        pass\n    import os\n    pass\n    run()\n",
        PY,
    );
    assert_eq!(titles(&graph), ["go", "Call run"]);
    assert!(runs_forward(&graph));
}

#[test]
fn reads_wire_back_to_assignments_and_parameters() {
    let graph = flow(
        "def go(a, b: int = 2, *rest, **opts):\n    x = a + b\n    log(x, rest, opts, key=x)\n    first, second = pair()\n    save(first, a.field)\n    x = 5\n    show(x)\n",
        PY,
    );
    let wires = wires(&graph);
    assert!(wires.contains(&("go", "a", "Set x")));
    assert!(wires.contains(&("go", "b", "Set x")));
    assert!(wires.contains(&("Set x", "x", "Call log")));
    assert!(wires.contains(&("go", "rest", "Call log")));
    assert!(wires.contains(&("go", "opts", "Call log")));
    assert!(wires.contains(&("Set first, second", "first", "Call save")));
    assert!(wires.contains(&("go", "a", "Call save")));
    // The latest binding wins.
    let show = wires
        .iter()
        .filter(|(_, name, to)| *name == "x" && *to == "Call show")
        .count();
    assert_eq!(show, 1);
    assert!(
        !wires
            .iter()
            .any(|(_, name, _)| *name == "key" || *name == "field")
    );
}

#[test]
fn inner_bindings_hide_outer_ones() {
    let graph = flow(
        "def go(item, items):\n    for item in items:\n        use(item)\n    squares = [item * item for item in items]\n    f = lambda item: item + 1\n    try:\n        a()\n    except Exception as item:\n        log(item)\n    done(item)\n",
        PY,
    );
    let wires = wires(&graph);
    assert!(!wires.contains(&("go", "item", "Call use")));
    assert!(!wires.contains(&("go", "item", "Set squares")));
    assert!(!wires.contains(&("go", "item", "Set f")));
    assert!(!wires.contains(&("go", "item", "Call log")));
    assert!(wires.contains(&("go", "items", "Set squares")));
    assert!(wires.contains(&("go", "item", "Call done")));
}

#[test]
fn finds_callers_in_the_file() {
    let source = "def save():\n    pass\n\nclass Store:\n    def flush(self):\n        save()\n        self.save()\n\ndef unrelated():\n    save_all()\n";
    let found = functions(source, PY);
    let callers: Vec<_> = callers_in(source, PY, &found[0])
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(callers, ["Store.flush", "Store.flush"]);
}

#[test]
fn a_methods_flow() {
    let source = "class Store:\n    def save(self, item):\n        if not item:\n            return\n        self.items.append(item)\n";
    let graph = flow_of(source, PY, "Store.save");
    assert_eq!(
        titles(&graph),
        ["Store.save", "Branch", "Return", "Call append"]
    );
    assert_eq!(after(&graph, "Branch", 1), ["Call append"]);
}
