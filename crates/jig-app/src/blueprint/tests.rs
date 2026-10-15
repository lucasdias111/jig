use super::*;

fn flow(source: &str) -> Graph {
    let functions = functions(source);
    graph(source, &functions[0]).expect("the function is there")
}

fn titles(graph: &Graph) -> Vec<&str> {
    graph.nodes.iter().map(|node| node.title.as_str()).collect()
}

/// The node that output `out` of `from` leads to.
fn next(graph: &Graph, from: usize, out: usize) -> Vec<usize> {
    graph
        .edges
        .iter()
        .filter(|edge| edge.from == from && edge.out == out)
        .map(|edge| edge.to)
        .collect()
}

#[test]
fn lists_functions_with_their_owners() {
    let source = "fn free() {}\n\
                  struct A;\n\
                  impl<T> Wrapper<T> { fn get(&self) {} }\n\
                  trait Named { fn name(&self) -> String { String::new() } fn bare(&self); }\n\
                  mod inner { fn hidden() {} }\n";
    let names: Vec<_> = functions(source).into_iter().map(|f| f.name).collect();
    assert_eq!(
        names,
        ["free", "Wrapper::get", "Named::name", "inner::hidden"]
    );
}

#[test]
fn picks_the_innermost_function_at_the_cursor() {
    let source = "fn a() {}\nfn b() { let x = 1; }\n";
    let found = functions(source);
    assert_eq!(at(&found, source.find("let").unwrap()), Some(1));
    // Between functions, the first.
    assert_eq!(at(&found, 9), Some(0));
    assert_eq!(at(&[], 0), None);
}

#[test]
fn statements_run_in_order_from_the_entry() {
    let graph = flow(
        "fn go(a: u8, b: &str) -> u8 {\n    let x = a + 1;\n    log(b);\n    self.items.push(x);\n    x\n}\n",
    );
    assert_eq!(
        titles(&graph),
        ["go", "Let x", "Call log", "Call push", "Return"]
    );
    assert_eq!(graph.nodes[0].code, ["a: u8", "b: &str", "→ u8"]);
    for ix in 0..4 {
        assert_eq!(next(&graph, ix, 0), [ix + 1]);
    }
    assert_eq!(graph.nodes[4].kind, Kind::Exit);
    assert!(graph.nodes[4].outs.is_empty());
}

#[test]
fn ranges_point_at_the_code() {
    let source = "fn go() {\n    work();\n}\n";
    let graph = flow(source);
    assert_eq!(&source[graph.nodes[1].range.clone()], "work();");
    assert_eq!(&source[graph.nodes[0].range.clone()], "fn go() ");
}

#[test]
fn if_branches_and_joins() {
    let graph = flow(
        "fn go() {\n    if ready() {\n        a();\n    } else {\n        b();\n    }\n    done();\n}\n",
    );
    assert_eq!(
        titles(&graph),
        ["go", "Branch", "Call a", "Call b", "Call done"]
    );
    assert_eq!(graph.nodes[1].outs, ["True", "False"]);
    assert_eq!(graph.nodes[1].code, ["ready()"]);
    assert_eq!(next(&graph, 1, 0), [2]);
    assert_eq!(next(&graph, 1, 1), [3]);
    // Both sides go on to `done`.
    assert_eq!(next(&graph, 2, 0), [4]);
    assert_eq!(next(&graph, 3, 0), [4]);
}

#[test]
fn if_without_else_goes_on_from_false() {
    let graph = flow("fn go() {\n    if x { return; }\n    after();\n}\n");
    assert_eq!(titles(&graph), ["go", "Branch", "Return", "Call after"]);
    assert_eq!(next(&graph, 1, 1), [3]);
    // Returning ends the path.
    assert!(next(&graph, 2, 0).is_empty());
}

#[test]
fn else_if_chains_from_false() {
    let graph = flow("fn go() {\n    if a { x(); } else if b { y(); }\n}\n");
    assert_eq!(
        titles(&graph),
        ["go", "Branch", "Call x", "Branch", "Call y"]
    );
    assert_eq!(next(&graph, 1, 1), [3]);
}

#[test]
fn match_has_an_output_per_arm() {
    let graph = flow(
        "fn go(v: Option<u8>) -> u8 {\n    match v {\n        Some(n) => n,\n        None => {\n            warn();\n            0\n        }\n    }\n}\n",
    );
    assert_eq!(graph.nodes[1].title, "Match");
    assert_eq!(graph.nodes[1].outs, ["Some(n)", "None"]);
    assert_eq!(
        titles(&graph),
        ["go", "Match", "Return", "Call warn", "Return"]
    );
    assert_eq!(next(&graph, 1, 1), [3]);
}

#[test]
fn loops_continue_from_completed_and_breaks() {
    let graph = flow(
        "fn go() {\n    for item in items {\n        if item.bad() { break; }\n        use_it(item);\n    }\n    after();\n}\n",
    );
    assert_eq!(
        titles(&graph),
        [
            "go",
            "For Each",
            "Branch",
            "Break",
            "Call use_it",
            "Call after"
        ]
    );
    assert_eq!(graph.nodes[1].code, ["item in items"]);
    assert_eq!(graph.nodes[1].outs, ["Body", "Completed"]);
    let mut into_after: Vec<_> = graph
        .edges
        .iter()
        .filter(|edge| edge.to == 5)
        .map(|edge| (edge.from, edge.out))
        .collect();
    into_after.sort();
    assert_eq!(into_after, [(1, 1), (3, 0)]);
    // The body's end loops back, which isn't drawn.
    assert!(next(&graph, 4, 0).is_empty());
}

#[test]
fn a_bare_loop_ends_only_by_breaking() {
    let graph = flow("fn go() {\n    loop { spin(); }\n    never();\n}\n");
    assert_eq!(graph.nodes[1].outs, ["Body"]);
    // Nothing reaches the code after it.
    assert!(graph.edges.iter().all(|edge| edge.to != 3));
}

#[test]
fn panics_and_tries_read_as_what_they_do() {
    let graph = flow(
        "fn go() -> Result<()> {\n    let file = open(path)?;\n    read(file).await?;\n    todo!()\n}\n",
    );
    assert_eq!(titles(&graph), ["go", "Let file", "Call read", "Panic"]);
}

#[test]
fn nested_items_and_comments_are_skipped() {
    let graph =
        flow("fn go() {\n    // why\n    use std::fmt;\n    fn helper() {}\n    run();\n}\n");
    assert_eq!(titles(&graph), ["go", "Call run"]);
}

#[test]
fn edges_run_forward() {
    let graph = flow(
        "fn go() {\n    while a() {\n        match b { 1 => c(), _ => continue }\n    }\n    loop { if d { break; } }\n}\n",
    );
    assert!(graph.edges.iter().all(|edge| edge.from < edge.to));
}

#[test]
fn long_code_is_cut_to_a_few_lines() {
    let graph = flow("fn go() {\n    call(\n        a,\n        b,\n        c,\n    );\n}\n");
    assert_eq!(graph.nodes[1].code, ["call(", "a,", "b, …"]);
}

/// Each data wire as (binding block's title, name, reading block's title).
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
fn reads_wire_back_to_lets_and_parameters() {
    let graph = flow(
        "fn go(path: &str, retries: u8) {\n    let file = open(path);\n    if retries > 0 {\n        read(file, retries);\n    }\n}\n",
    );
    assert_eq!(
        wires(&graph),
        [
            ("go", "path", "Let file"),
            ("go", "retries", "Branch"),
            ("Let file", "file", "Call read"),
            ("go", "retries", "Call read"),
        ]
    );
    assert_eq!(graph.nodes[0].values, ["path", "retries"]);
    assert_eq!(graph.nodes[3].inputs, ["file", "retries"]);
    let read = graph.data.iter().filter(|data| data.to == 3);
    assert_eq!(read.map(|data| data.input).collect::<Vec<_>>(), [0, 1]);
}

#[test]
fn the_latest_binding_wins() {
    let graph = flow("fn go(x: u8) {\n    let x = x + 1;\n    let x = x * 2;\n    show(x);\n}\n");
    assert_eq!(
        wires(&graph),
        [
            ("go", "x", "Let x"),
            ("Let x", "x", "Let x"),
            ("Let x", "x", "Call show"),
        ]
    );
    // Each `x` comes from the one just before it.
    let froms: Vec<_> = graph.data.iter().map(|data| (data.from, data.to)).collect();
    assert_eq!(froms, [(0, 1), (1, 2), (2, 3)]);
}

#[test]
fn inner_bindings_hide_outer_ones() {
    let graph = flow(
        "fn go(item: u8, list: Vec<u8>) {\n    for item in list {\n        use_it(item);\n    }\n    if let Some(item) = first() {\n        keep(item);\n    }\n    match pick() {\n        Some(item) => drop(item),\n        None => drop(item),\n    }\n}\n",
    );
    assert_eq!(
        wires(&graph),
        [
            ("go", "list", "For Each"),
            // Only the last arm, the function's value, sees the parameter.
            ("go", "item", "Return"),
        ]
    );
    let last = graph.nodes.len() - 1;
    assert_eq!(graph.data[1].to, last);
}

#[test]
fn closures_read_what_they_capture_but_not_their_parameters() {
    let graph = flow(
        "fn go(scale: u8, n: u8) {\n    let double = |n| n * scale;\n    apply(double, n);\n}\n",
    );
    assert_eq!(
        wires(&graph),
        [
            ("go", "scale", "Let double"),
            ("Let double", "double", "Call apply"),
            ("go", "n", "Call apply"),
        ]
    );
}

#[test]
fn a_let_inside_a_block_hides_until_the_block_ends() {
    let graph = flow(
        "fn go(x: u8) {\n    let y = {\n        let x = 2;\n        x\n    };\n    sum(x, y);\n}\n",
    );
    assert_eq!(
        wires(&graph),
        [("go", "x", "Call sum"), ("Let y", "y", "Call sum")]
    );
}

#[test]
fn fields_paths_and_macros() {
    let graph = flow(
        "fn go(len: usize, items: Vec<u8>) {\n    self.len.max(items.len);\n    std::len::make();\n    let point = Point { len, x: 1 };\n    println!(\"{}\", point.items);\n}\n",
    );
    assert_eq!(
        wires(&graph),
        [
            ("go", "items", "Call max"),
            ("go", "len", "Let point"),
            ("Let point", "point", "println!"),
        ]
    );
}

#[test]
fn bindings_nobody_reads_have_no_pin() {
    let graph = flow("fn go(unused: u8) {\n    let a = 1;\n    let b = a;\n}\n");
    assert!(graph.nodes[0].values.is_empty());
    assert_eq!(graph.nodes[1].values, ["a"]);
    assert!(graph.nodes[2].values.is_empty());
    assert!(graph.data.iter().all(|data| data.from < data.to));
}

#[test]
fn call_blocks_know_what_they_call() {
    let source = "fn go() {\n    let file = fs::open(path)?;\n    self.items.push(file);\n    println!(\"x\");\n    total += 1;\n}\n";
    let graph = flow(source);
    let callee = |ix: usize| graph.nodes[ix].callee.map(|at| &source[at..at + 4]);
    assert_eq!(callee(1), Some("open"));
    assert_eq!(callee(2), Some("push"));
    // Macros and assignments call nothing to open.
    assert_eq!(callee(3), None);
    assert_eq!(callee(4), None);
}

#[test]
fn functions_know_where_their_name_is() {
    let source = "impl A {\n    pub fn save(&self) {}\n}\n";
    let save = &functions(source)[0];
    assert_eq!(&source[save.name_at..save.name_at + 4], "save");
}

#[test]
fn finds_callers_in_the_file() {
    let source = "fn save() {}\n\
                  fn on_click() {\n    save();\n}\n\
                  impl Doc {\n    fn close(&self) {\n        self.save();\n        other();\n    }\n}\n\
                  fn save_as() {}\n";
    let found = functions(source);
    let save = found.iter().find(|f| f.name == "save").unwrap();
    let callers: Vec<_> = callers_in(source, save)
        .into_iter()
        .map(|(name, at)| (name, &source[at..at + 4]))
        .collect();
    assert_eq!(
        callers,
        [
            ("on_click".to_string(), "save"),
            ("Doc::close".to_string(), "save")
        ]
    );
}

#[test]
fn the_block_at_an_offset_is_the_innermost() {
    let source = "fn go() {\n    if ready {\n        work();\n    }\n}\n";
    let graph = flow(source);
    let at = node_at(&graph, source.find("work").unwrap()).unwrap();
    assert_eq!(graph.nodes[at].title, "Call work");
}

#[test]
fn functions_are_grouped_by_what_they_are_in() {
    let source = "fn free() {}\n\
                  impl<T> Wrapper<T> { fn get(&self) {} }\n\
                  impl fmt::Display for Wrapper<u8> { fn fmt(&self) {} }\n\
                  trait Named { fn name(&self) {} }\n\
                  mod tests { struct S; impl S { fn check() {} } fn helper() {} }\n";
    let listed: Vec<_> = functions(source)
        .into_iter()
        .map(|f| (f.section, f.short, f.name))
        .collect();
    let row = |section: Option<&str>, short: &str, name: &str| {
        (
            section.map(str::to_string),
            short.to_string(),
            name.to_string(),
        )
    };
    assert_eq!(
        listed,
        [
            row(None, "free", "free"),
            row(Some("Wrapper"), "get", "Wrapper::get"),
            row(Some("fmt::Display for Wrapper"), "fmt", "Wrapper::fmt"),
            row(Some("trait Named"), "name", "Named::name"),
            row(Some("mod tests › S"), "check", "tests::S::check"),
            row(Some("mod tests"), "helper", "tests::helper"),
        ]
    );
}
