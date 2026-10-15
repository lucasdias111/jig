use crate::blueprint::lang::testing::{after, flow, node, runs_forward, titles, wires};
use crate::blueprint::{Kind, Lang, callers_in, functions};

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

#[test]
fn statements_run_in_order_from_the_entry() {
    let graph = flow(
        "package main\nfunc (s *Store) Save(item int, names ...string) (int, error) {\n\tx := compute(item)\n\tvar y, z = 1, 2\n\tconst limit = 3\n\ts.items = append(s.items, x)\n\tx++\n\tdefer done()\n\tgo run(x)\n\treturn x, nil\n}\n",
        GO,
    );
    assert_eq!(
        titles(&graph),
        [
            "Store.Save",
            "Let x",
            "Var y, z",
            "Const limit",
            "Set s.items",
            "Set x",
            "Defer done",
            "Go run",
            "Return"
        ]
    );
    assert_eq!(
        graph.nodes[0].code,
        ["s *Store", "item int", "names ...string …"],
        "the receiver first"
    );
    assert_eq!(graph.nodes[1].kind, Kind::Let);
    assert!(graph.nodes[1].callee.is_some());
    assert!(
        graph.nodes[6].callee.is_some(),
        "a defer knows what it calls"
    );
}

#[test]
fn if_with_a_statement_chains_else_if() {
    let graph = flow(
        "package main\nfunc go(x int) {\n\tif v := x; v > 2 {\n\t\tbig(v)\n\t} else if x < 0 {\n\t\tneg()\n\t} else {\n\t\tsmall()\n\t}\n\tdone()\n}\n",
        GO,
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
    assert_eq!(graph.nodes[1].code, ["v := x; v > 2"]);
    assert_eq!(after(&graph, "Branch", 1), ["Branch"]);
    for from in ["Call big", "Call neg", "Call small"] {
        assert_eq!(after(&graph, from, 0), ["Call done"]);
    }
}

#[test]
fn the_four_kinds_of_for() {
    let graph = flow(
        "package main\nfunc go(items []int) {\n\tfor i, v := range items {\n\t\tif v > 2 {\n\t\t\tbreak\n\t\t}\n\t\tuse(i)\n\t}\n\tfor i := 0; i < 3; i++ {\n\t\ttick(i)\n\t}\n\tfor busy() {\n\t\twait()\n\t}\n\tfor {\n\t\tspin()\n\t}\n}\n",
        GO,
    );
    let each = node(&graph, "For Each");
    assert_eq!(graph.nodes[each].code, ["i, v := range items"]);
    assert_eq!(after(&graph, "For Each", 1), ["For"]);
    assert_eq!(after(&graph, "Break", 0), ["For"]);
    assert_eq!(
        graph.nodes[node(&graph, "For")].code,
        ["i := 0; i < 3; i++"]
    );
    assert_eq!(graph.nodes[node(&graph, "While")].code, ["busy()"]);
    let bare = node(&graph, "Loop");
    assert_eq!(graph.nodes[bare].outs, ["Body"], "only a break leaves it");
}

#[test]
fn switch_runs_on_only_with_fallthrough() {
    let graph = flow(
        "package main\nfunc go(x int) {\n\tswitch x {\n\tcase 1, 2:\n\t\tone()\n\t\tfallthrough\n\tcase 3:\n\t\tthree()\n\tdefault:\n\t\tother()\n\t}\n\tdone()\n}\n",
        GO,
    );
    let switch = node(&graph, "Switch");
    assert_eq!(graph.nodes[switch].outs, ["1, 2", "3", "default"]);
    assert_eq!(graph.nodes[switch].code, ["x"]);
    assert_eq!(after(&graph, "Call one", 0), ["Call three"]);
    assert_eq!(after(&graph, "Call three", 0), ["Call done"]);
    assert_eq!(after(&graph, "Call other", 0), ["Call done"]);
}

#[test]
fn a_switch_without_default_and_a_select() {
    let graph = flow(
        "package main\nfunc go(x int, ch chan int) {\n\tswitch {\n\tcase x > 1:\n\t\ta()\n\t}\n\tselect {\n\tcase v := <-ch:\n\t\tuse(v)\n\tdefault:\n\t\tidle()\n\t}\n\tdone()\n}\n",
        GO,
    );
    let switch = node(&graph, "Switch");
    assert_eq!(graph.nodes[switch].outs, ["x > 1", "Default"]);
    assert_eq!(after(&graph, "Switch", 1), ["Select"]);
    let select = node(&graph, "Select");
    assert_eq!(graph.nodes[select].outs, ["v := <-ch", "default"]);
    assert_eq!(after(&graph, "Call use", 0), ["Call done"]);
}

#[test]
fn panic_and_goto_end_a_path() {
    let graph = flow(
        "package main\nfunc go(ok bool) {\n\tif !ok {\n\t\tpanic(\"no\")\n\t}\n\tgoto out\nout:\n\tdone()\n}\n",
        GO,
    );
    assert_eq!(graph.nodes[node(&graph, "Panic")].kind, Kind::Exit);
    assert_eq!(graph.nodes[node(&graph, "Goto")].kind, Kind::Exit);
    assert!(runs_forward(&graph));
}

#[test]
fn reads_wire_back_to_declarations_and_parameters() {
    let graph = flow(
        "package main\nfunc (s *Store) go(a int, b string) {\n\tx := a + 1\n\tvar y = x * 2\n\tlog(x, y, b, s.name)\n\tx = 5\n\tf := func(q int) int { return q + x }\n\tfor i, v := range s.items {\n\t\tuse(i, v)\n\t}\n\tsave(f)\n}\n",
        GO,
    );
    let wires = wires(&graph);
    assert!(wires.contains(&("Store.go", "a", "Let x")));
    assert!(wires.contains(&("Let x", "x", "Var y")));
    assert!(wires.contains(&("Var y", "y", "Call log")));
    assert!(wires.contains(&("Store.go", "b", "Call log")));
    assert!(wires.contains(&("Store.go", "s", "Call log")));
    assert!(
        wires.contains(&("Let x", "x", "Let f")),
        "a closure reads what it captures"
    );
    assert!(wires.contains(&("Store.go", "s", "For Each")));
    assert!(wires.contains(&("Let f", "f", "Call save")));
    assert!(
        !wires
            .iter()
            .any(|(_, name, _)| *name == "name" || *name == "q")
    );
}

#[test]
fn finds_callers_in_the_file() {
    let source = "package main\nfunc save() {}\nfunc (s *Store) Flush() {\n\tsave()\n\ts.save()\n}\nfunc other() { saveAll() }\n";
    let found = functions(source, GO);
    let callers: Vec<_> = callers_in(source, GO, &found[0])
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(callers, ["Store.Flush", "Store.Flush"]);
}
