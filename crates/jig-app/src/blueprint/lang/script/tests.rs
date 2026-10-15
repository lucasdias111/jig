use crate::blueprint::{Graph, Kind, Lang, at, callers_in, functions, graph};

fn flow_in(source: &str, lang: Lang) -> Graph {
    let functions = functions(source, lang);
    graph(source, lang, &functions[0]).expect("the function is there")
}

fn flow(source: &str) -> Graph {
    flow_in(source, Lang::TypeScript)
}

fn titles(graph: &Graph) -> Vec<&str> {
    graph.nodes.iter().map(|node| node.title.as_str()).collect()
}

/// The nodes output `out` of `from` leads to.
fn next(graph: &Graph, from: usize, out: usize) -> Vec<usize> {
    graph
        .edges
        .iter()
        .filter(|edge| edge.from == from && edge.out == out)
        .map(|edge| edge.to)
        .collect()
}

fn node(graph: &Graph, title: &str) -> usize {
    graph
        .nodes
        .iter()
        .position(|node| node.title == title)
        .unwrap_or_else(|| panic!("no {title} in {:?}", titles(graph)))
}

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
    assert_eq!(at(&found, source.find("go()").unwrap()), Some(3));
}

#[test]
fn statements_run_in_order_from_the_entry() {
    let graph = flow(
        "function go(a: number, b?: string): number {\n  const x = a + 1;\n  let y = compute(x), z = 2;\n  log(b);\n  this.items.push(x);\n  x += 1;\n  i++;\n  new Thing(x);\n  await save();\n  return x;\n}\n",
    );
    assert_eq!(
        titles(&graph),
        [
            "go",
            "Const x",
            "Let y, z",
            "Call log",
            "Call push",
            "Set x",
            "Set i",
            "New Thing",
            "Call save",
            "Return"
        ]
    );
    assert_eq!(graph.nodes[0].code, ["a: number", "b?: string", "→ number"]);
    for ix in 0..9 {
        assert_eq!(next(&graph, ix, 0), [ix + 1]);
    }
    assert_eq!(graph.nodes[1].kind, Kind::Let);
    assert_eq!(graph.nodes[9].kind, Kind::Exit);
    assert!(graph.nodes[2].callee.is_some(), "a let knows what it calls");
}

#[test]
fn ranges_point_at_the_code() {
    let source = "function go() {\n  work();\n}\n";
    let graph = flow_in(source, Lang::JavaScript);
    assert_eq!(&source[graph.nodes[1].range.clone()], "work();");
    assert_eq!(&source[graph.nodes[0].range.clone()], "function go() ");
}

#[test]
fn an_arrow_with_an_expression_returns_it() {
    let graph = flow("const double = (n: number) => n * 2;\n");
    assert_eq!(titles(&graph), ["double", "Return"]);
    assert_eq!(graph.nodes[0].code, ["n: number"]);
    let graph = flow_in("const twice = n => n * 2;\n", Lang::JavaScript);
    assert_eq!(graph.nodes[0].code, ["n"]);
}

#[test]
fn if_branches_joins_and_chains_else_if() {
    let graph = flow(
        "function go() {\n  if (ready()) {\n    a();\n  } else if (later) b();\n  else {\n    c();\n  }\n  done();\n}\n",
    );
    assert_eq!(
        titles(&graph),
        [
            "go",
            "Branch",
            "Call a",
            "Branch",
            "Call b",
            "Call c",
            "Call done"
        ]
    );
    assert_eq!(graph.nodes[1].code, ["ready()"]);
    assert_eq!(next(&graph, 1, 0), [2]);
    assert_eq!(next(&graph, 1, 1), [3]);
    assert_eq!(next(&graph, 3, 0), [4]);
    assert_eq!(next(&graph, 3, 1), [5]);
    for from in [2, 4, 5] {
        assert_eq!(next(&graph, from, 0), [6]);
    }
}

#[test]
fn switch_falls_through_until_a_break() {
    let graph = flow(
        "function go(key: string) {\n  switch (key) {\n    case 'a':\n      one();\n    case 'b':\n      two();\n      break;\n    default:\n      other();\n  }\n  done();\n}\n",
    );
    assert_eq!(
        titles(&graph),
        [
            "go",
            "Switch",
            "Call one",
            "Call two",
            "Break",
            "Call other",
            "Call done"
        ]
    );
    let switch = node(&graph, "Switch");
    assert_eq!(graph.nodes[switch].outs, ["'a'", "'b'", "default"]);
    assert_eq!(graph.nodes[switch].code, ["key"]);
    // 'a' runs on into 'b'.
    assert_eq!(
        next(&graph, node(&graph, "Call one"), 0),
        [node(&graph, "Call two")]
    );
    assert_eq!(next(&graph, switch, 1), [node(&graph, "Call two")]);
    let done = node(&graph, "Call done");
    assert_eq!(next(&graph, node(&graph, "Break"), 0), [done]);
    assert_eq!(next(&graph, node(&graph, "Call other"), 0), [done]);
}

#[test]
fn a_switch_without_default_goes_on() {
    let graph = flow(
        "function go(n: number) {\n  switch (n) {\n    case 1: one(); break;\n  }\n  done();\n}\n",
    );
    let switch = node(&graph, "Switch");
    assert_eq!(graph.nodes[switch].outs, ["1", "Default"]);
    assert_eq!(next(&graph, switch, 1), [node(&graph, "Call done")]);
}

#[test]
fn loops_continue_from_completed_and_breaks() {
    let graph = flow(
        "function go(items: number[]) {\n  for (const item of items) {\n    if (item > 2) break;\n    if (item < 0) continue;\n    use(item);\n  }\n  for (let i = 0; i < 3; i++) tick(i);\n  while (busy()) wait();\n  do { spin(); } while (again);\n  done();\n}\n",
    );
    let each = node(&graph, "For Each");
    assert_eq!(graph.nodes[each].code, ["const item of items"]);
    assert_eq!(graph.nodes[each].outs, ["Body", "Completed"]);
    let after_each = next(&graph, each, 1);
    assert_eq!(after_each, [node(&graph, "For")]);
    assert_eq!(
        next(&graph, node(&graph, "Break"), 0),
        [node(&graph, "For")],
        "a break goes on after the loop"
    );
    assert_eq!(graph.nodes[node(&graph, "Continue")].kind, Kind::Exit);
    assert_eq!(
        graph.nodes[node(&graph, "For")].code,
        ["let i = 0; i < 3; i++"]
    );
    assert_eq!(graph.nodes[node(&graph, "While")].code, ["busy()"]);
    let doing = node(&graph, "Do While");
    assert_eq!(graph.nodes[doing].code, ["again"]);
    assert_eq!(next(&graph, doing, 0), [node(&graph, "Call spin")]);
    assert_eq!(next(&graph, doing, 1), [node(&graph, "Call done")]);
}

#[test]
fn try_runs_its_body_or_catch_then_finally() {
    let graph = flow(
        "async function go() {\n  try {\n    await risky();\n  } catch (error) {\n    report(error);\n  } finally {\n    cleanUp();\n  }\n  throw new Error('no');\n}\n",
    );
    let attempt = node(&graph, "Try");
    assert_eq!(graph.nodes[attempt].outs, ["Body", "Catch"]);
    assert_eq!(next(&graph, attempt, 0), [node(&graph, "Call risky")]);
    assert_eq!(next(&graph, attempt, 1), [node(&graph, "Call report")]);
    let finally = node(&graph, "Call cleanUp");
    assert_eq!(next(&graph, node(&graph, "Call risky"), 0), [finally]);
    assert_eq!(next(&graph, node(&graph, "Call report"), 0), [finally]);
    assert_eq!(next(&graph, finally, 0), [node(&graph, "Throw")]);
    assert_eq!(graph.nodes[node(&graph, "Throw")].kind, Kind::Exit);
}

#[test]
fn nested_declarations_and_comments_are_skipped() {
    let graph = flow(
        "function go() {\n  // a note\n  function helper() {}\n  type T = number;\n  run();\n}\n",
    );
    assert_eq!(titles(&graph), ["go", "Call run"]);
}

#[test]
fn edges_run_forward() {
    let graph = flow(
        "function go(n: number) {\n  for (const x of xs) {\n    switch (n) { case 1: if (x) break; default: a(); }\n  }\n  try { b(); } catch { c(); }\n}\n",
    );
    assert!(graph.edges.iter().all(|edge| edge.from < edge.to));
}

fn wires(graph: &Graph) -> Vec<(&str, &str, &str)> {
    graph
        .data
        .iter()
        .map(|data| {
            (
                graph.nodes[data.from].title.as_str(),
                graph.nodes[data.from].values[data.value].as_str(),
                graph.nodes[data.to].title.as_str(),
            )
        })
        .collect()
}

#[test]
fn reads_wire_back_to_declarations_and_parameters() {
    let graph = flow(
        "function go(a: number, { b, c: renamed }: Opts, ...rest: number[]) {\n  const x = a + b;\n  log(x, renamed, rest);\n  const [first, second = a] = list;\n  save({ first, x });\n}\n",
    );
    let wires = wires(&graph);
    assert!(wires.contains(&("go", "a", "Const x")));
    assert!(wires.contains(&("go", "b", "Const x")));
    assert!(wires.contains(&("Const x", "x", "Call log")));
    assert!(wires.contains(&("go", "renamed", "Call log")));
    assert!(wires.contains(&("go", "rest", "Call log")));
    // A default reads what's around it; a shorthand property reads too.
    assert!(wires.contains(&("go", "a", "Const [first, second = a]")));
    assert!(wires.contains(&("Const [first, second = a]", "first", "Call save")));
    assert!(wires.contains(&("Const x", "x", "Call save")));
    assert!(
        !wires.iter().any(|(_, name, _)| *name == "c"),
        "a key, not a name"
    );
}

#[test]
fn inner_bindings_hide_outer_ones() {
    let graph = flow(
        "function go(item: number) {\n  for (const item of items) use(item);\n  list.map((item) => item * 2);\n  try { a(); } catch (item) { log(item); }\n  if (ok) {\n    const item = 2;\n    show(item);\n  }\n  done(item);\n}\n",
    );
    let wires = wires(&graph);
    assert!(!wires.contains(&("go", "item", "Call use")));
    assert!(!wires.contains(&("go", "item", "Call map")));
    assert!(!wires.contains(&("go", "item", "Call log")));
    assert!(wires.contains(&("Const item", "item", "Call show")));
    assert!(wires.contains(&("go", "item", "Call done")));
}

#[test]
fn closures_read_what_they_capture() {
    let graph = flow("function go(limit: number) {\n  items.filter((x) => x < limit);\n}\n");
    assert_eq!(wires(&graph), [("go", "limit", "Call filter")]);
}

#[test]
fn types_and_members_are_not_reads() {
    let graph = flow(
        "function go(x: number) {\n  const y: typeof x = x;\n  this.x = y.x;\n  const o = { x: 1 };\n}\n",
    );
    let wires = wires(&graph);
    assert!(wires.contains(&("go", "x", "Const y")));
    assert!(wires.contains(&("Const y", "y", "Set this.x")));
    assert!(
        !wires.contains(&("go", "x", "Set this.x")),
        "a property, not x"
    );
    assert!(!wires.contains(&("go", "x", "Const o")), "a key, not x");
}

#[test]
fn finds_callers_in_the_file() {
    let source = "function save() {}\n\
                  class Store {\n  flush() { save(); this.save(); }\n}\n\
                  const later = () => { save(); };\n\
                  function unrelated() { saveAll(); }\n";
    let found = functions(source, Lang::JavaScript);
    let callers: Vec<_> = callers_in(source, Lang::JavaScript, &found[0])
        .into_iter()
        .map(|(name, _)| name)
        .collect();
    assert_eq!(callers, ["Store.flush", "Store.flush", "later"]);
}

#[test]
fn reads_jsx_in_javascript_and_tsx() {
    let source = "export function App({ name }) {\n  const greeting = `Hi ${name}`;\n  return <div title={greeting}>{name}</div>;\n}\n";
    for lang in [Lang::JavaScript, Lang::Tsx] {
        let graph = flow_in(source, lang);
        assert_eq!(titles(&graph), ["App", "Const greeting", "Return"]);
        let wires = wires(&graph);
        assert!(wires.contains(&("App", "name", "Const greeting")));
        assert!(wires.contains(&("Const greeting", "greeting", "Return")));
        assert!(wires.contains(&("App", "name", "Return")));
    }
}
