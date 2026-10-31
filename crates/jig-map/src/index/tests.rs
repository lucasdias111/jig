use super::*;

pub use super::fixture::fixture;

pub fn id(project: &Project, title: &str) -> FnId {
    project
        .functions
        .iter()
        .position(|f| f.title() == title)
        .unwrap_or_else(|| panic!("no {title}"))
}

fn targets(project: &Project, title: &str) -> Vec<String> {
    project.edges[id(project, title)]
        .iter()
        .map(|edge| match &edge.target {
            Target::Function(to) => project.functions[*to].title(),
            Target::Ambiguous(ids) => format!("~{}", ids.len()),
            Target::External(package) => format!("ext {package}"),
            Target::Unknown => "?".into(),
        })
        .collect()
}

#[test]
fn reads_modules_and_skips_dependencies() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let labels: Vec<&str> = project.modules.iter().map(|m| m.label.as_str()).collect();
    assert_eq!(
        labels,
        ["client/src/net", "client/src", "demo.bugs", "demo.users"]
    );
    assert!(
        !project
            .files
            .iter()
            .any(|f| f.path.starts_with("client/node_modules"))
    );
}

#[test]
fn java_calls_follow_types_imports_and_interfaces() {
    let dir = fixture();
    let project = Project::read(dir.path());
    assert_eq!(
        targets(&project, "BugService.create"),
        [
            "UserService.find",
            "BugService.save",
            "ext org.slf4j",
            "Notifier.send"
        ]
    );
    let send = id(&project, "Notifier.send");
    let implementations: Vec<String> = project
        .implementations(send)
        .into_iter()
        .map(|id| project.functions[id].title())
        .collect();
    assert_eq!(implementations, ["MailNotifier.send"]);
    let find = id(&project, "UserService.find");
    assert_eq!(project.callers[find], [id(&project, "BugService.create")]);
}

#[test]
fn typescript_calls_follow_imports_and_parameters() {
    let dir = fixture();
    let project = Project::read(dir.path());
    assert_eq!(
        targets(&project, "join"),
        ["connect", "ext mediasoup", "send", "?"]
    );
    let rooms = project
        .modules
        .iter()
        .position(|m| m.label == "client/src")
        .unwrap();
    let net = project
        .modules
        .iter()
        .position(|m| m.label == "client/src/net")
        .unwrap();
    assert_eq!(project.imports[rooms], [(net, 1)]);
    let bugs = project
        .modules
        .iter()
        .position(|m| m.label == "demo.bugs")
        .unwrap();
    let users = project
        .modules
        .iter()
        .position(|m| m.label == "demo.users")
        .unwrap();
    assert_eq!(project.imports[bugs], [(users, 1)]);
}

#[test]
fn symbols_are_found_again_after_a_rename() {
    let dir = fixture();
    let project = Project::read(dir.path());
    let mut symbol = project.symbol(id(&project, "UserService.find"));
    assert_eq!(
        project.find(&symbol),
        Some(id(&project, "UserService.find"))
    );
    symbol.file = "server/src/org/demo/users/Old.java".into();
    assert_eq!(
        project.find(&symbol),
        Some(id(&project, "UserService.find"))
    );
    symbol.name = "gone".into();
    assert_eq!(project.find(&symbol), None);
}

/// How a real project reads: `JIG_MAP_PROJECT=<folder>`, `--ignored
/// --nocapture`. Only reads it.
#[test]
#[ignore]
fn reads_a_real_project() {
    let root = std::env::var_os("JIG_MAP_PROJECT").expect("JIG_MAP_PROJECT");
    let started = std::time::Instant::now();
    let project = Project::read(Path::new(&root));
    let took = started.elapsed();
    let mut kinds = [0usize; 4];
    for edge in project.edges.iter().flatten() {
        kinds[match edge.target {
            Target::Function(_) => 0,
            Target::Ambiguous(_) => 1,
            Target::External(_) => 2,
            Target::Unknown => 3,
        }] += 1;
    }
    let hubs: Vec<String> = (0..project.functions.len())
        .filter(|&id| project.is_hub(id))
        .map(|id| project.functions[id].title())
        .collect();
    println!(
        "{} files, {} modules, {} functions in {took:?}; calls: {} certain, {} ambiguous, {} external, {} unknown; hubs: {hubs:?}",
        project.files.len(),
        project.modules.len(),
        project.functions.len(),
        kinds[0],
        kinds[1],
        kinds[2],
        kinds[3]
    );
    let mut externals: HashMap<&str, usize> = HashMap::new();
    for edge in project.edges.iter().flatten() {
        if let Target::External(package) = &edge.target {
            *externals.entry(package.as_str()).or_default() += 1;
        }
    }
    let mut externals: Vec<_> = externals.into_iter().collect();
    externals.sort_by_key(|(_, n)| std::cmp::Reverse(*n));
    println!("externals: {:?}", &externals[..externals.len().min(25)]);
    let imports: usize = project.imports.iter().map(Vec::len).sum();
    println!("module imports: {imports}");
    for module in project.modules.iter().take(80) {
        println!("  {} ({} fns)", module.label, module.functions.len());
    }
}
