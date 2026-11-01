use super::*;

const JAVA: &str = "package org.margin.bugs;\n\nimport org.margin.users.UserService;\nimport org.slf4j.Logger;\n\npublic class BugService implements Reporter {\n    private final UserService users;\n    private Logger log;\n\n    public Bug create(String title, long userId) {\n        User user = users.find(userId);\n        Bug bug = save(title, user);\n        log.info(\"made\");\n        items.forEach(this::notify);\n        return bug;\n    }\n\n    private Bug save(String title, User user) { return null; }\n    private void notify(Bug bug) {}\n}\n\ninterface Reporter {\n    Bug create(String title, long userId);\n}\n";

#[test]
fn java_functions_contracts_and_calls() {
    let outline = outline(JAVA, Lang::Java);
    assert_eq!(outline.package.as_deref(), Some("org.margin.bugs"));
    assert_eq!(outline.imports[0].path, "org.margin.users.UserService");
    let class = &outline.types[0];
    assert_eq!(class.name, "BugService");
    assert_eq!(class.implements, ["Reporter"]);
    assert!(
        class
            .fields
            .contains(&("users".into(), "UserService".into()))
    );
    let create = &outline.functions[0];
    assert_eq!(create.function.name, "BugService.create");
    assert_eq!(create.owner.as_deref(), Some("BugService"));
    let parameters: Vec<String> = create.parameters.iter().map(Parameter::label).collect();
    assert_eq!(parameters, ["title: String", "userId: long"]);
    assert_eq!(create.returns.as_deref(), Some("Bug"));
    // Names handed on are kept as possible callbacks; only those naming
    // a function count, which the index decides.
    let calls: Vec<(&str, Option<&str>, Option<&str>, bool)> = create
        .calls
        .iter()
        .filter(|c| !c.reference || c.name == "notify")
        .map(|c| {
            (
                c.name.as_str(),
                c.qualifier.as_deref(),
                c.bound.as_deref(),
                c.reference,
            )
        })
        .collect();
    assert_eq!(
        calls,
        [
            ("find", Some("users"), Some("user"), false),
            ("save", None, Some("bug"), false),
            ("info", Some("log"), None, false),
            ("forEach", Some("items"), None, false),
            ("notify", None, None, true),
        ]
    );
    let save = create.calls.iter().find(|c| c.name == "save").unwrap();
    assert_eq!(save.arguments, ["title", "user"]);
    let abstract_create = outline
        .functions
        .iter()
        .find(|f| f.is_abstract)
        .expect("the interface's method");
    assert_eq!(abstract_create.function.name, "Reporter.create");
    assert_eq!(abstract_create.owner.as_deref(), Some("Reporter"));
}

const TS: &str = "import { Room } from './types.js'\nimport * as mediasoup from 'mediasoup'\nimport helper, { join as enter } from '../helpers'\n\nexport async function getRoom(rooms: Map<string, Room>, id: string, done: () => void): Promise<Room> {\n    const router = await mediasoup.createRouter()\n    const room = makeRoom(router, id)\n    enter(room)\n    done()\n    return room\n}\n\nfunction makeRoom(router: Router, id: string): Room { return { router, id } }\n\nclass Hub implements Notifier {\n    constructor(private store: Store) {}\n    send(text: string): void { this.store.save(text) }\n}\n\ninterface Notifier { send(text: string): void }\n";

#[test]
fn typescript_imports_contracts_and_calls() {
    let outline = outline(TS, Lang::TypeScript);
    assert_eq!(outline.imports[0].path, "./types.js");
    assert_eq!(
        outline.imports[0].names,
        [("Room".to_string(), "Room".to_string())]
    );
    assert!(outline.imports[1].wildcard);
    assert_eq!(
        outline.imports[2].names,
        [
            ("helper".to_string(), "default".to_string()),
            ("enter".to_string(), "join".to_string())
        ]
    );
    let get = &outline.functions[0];
    assert_eq!(get.function.name, "getRoom");
    assert_eq!(get.returns.as_deref(), Some("Promise<Room>"));
    assert_eq!(get.parameters[0].label(), "rooms: Map<string, Room>");
    let calls: Vec<(&str, Option<&str>, bool)> = get
        .calls
        .iter()
        .filter(|c| !c.reference)
        .map(|c| (c.name.as_str(), c.qualifier.as_deref(), c.through_parameter))
        .collect();
    assert_eq!(
        calls,
        [
            ("createRouter", Some("mediasoup"), false),
            ("makeRoom", None, false),
            ("enter", None, false),
            ("done", None, true),
        ]
    );
    let hub = outline.types.iter().find(|t| t.name == "Hub").unwrap();
    assert_eq!(hub.implements, ["Notifier"]);
    assert!(hub.fields.contains(&("store".into(), "Store".into())));
    let send = outline
        .functions
        .iter()
        .find(|f| f.function.name == "Hub.send")
        .unwrap();
    assert_eq!(send.calls[0].qualifier.as_deref(), Some("store"));
    assert!(
        outline
            .functions
            .iter()
            .any(|f| f.is_abstract && f.function.name == "Notifier.send")
    );
}

#[test]
fn fields_and_variables_for_the_overview() {
    let fields = |source: &str, lang: Lang| -> Vec<String> {
        outline(source, lang)
            .variables
            .iter()
            .map(|v| match (&v.owner, &v.ty) {
                (Some(owner), Some(ty)) => format!("{owner}.{}: {ty}", v.name),
                (Some(owner), None) => format!("{owner}.{}", v.name),
                (None, Some(ty)) => format!("{}: {ty}", v.name),
                (None, None) => v.name.clone(),
            })
            .collect()
    };
    assert_eq!(
        fields(JAVA, Lang::Java),
        ["BugService.users: UserService", "BugService.log: Logger"]
    );
    assert_eq!(
        fields(
            "const LIMIT = 3\nexport let count: number = 0\nconst go = () => {}\nclass Hub { store: Store; run() { let local = 1 } }\n",
            Lang::TypeScript
        ),
        ["LIMIT", "count: number", "Hub.store: Store"]
    );
    assert_eq!(
        fields(
            "LIMIT = 3\n\nclass Hub:\n    size: int = 0\n    def run(self):\n        local = 1\n",
            Lang::Python
        ),
        ["LIMIT", "Hub.size: int"]
    );
    assert_eq!(
        fields(
            "package main\n\nvar count int\n\ntype Hub struct {\n\tstore Store\n}\n\nfunc f() { x := 1 }\n",
            Lang::Go
        ),
        ["count: int", "Hub.store: Store"]
    );
    assert_eq!(
        fields(
            "static LIMIT: u8 = 3;\nstruct Hub { store: Store }\nfn f() { let x = 1; }\n",
            Lang::Rust
        ),
        ["LIMIT: u8", "Hub.store: Store"]
    );
    assert_eq!(
        fields(
            "int count = 0;\nstruct hub { int size; };\nvoid f(void) { int x = 1; }\nint g(void);\n",
            Lang::C
        ),
        ["count: int", "hub.size: int"]
    );
}

#[test]
fn rust_calls_and_impls() {
    let source = "use crate::store::Store;\n\nstruct Hub { store: Store }\n\nimpl Notifier for Hub {\n    fn send(&self, text: &str) -> bool {\n        let saved = self.store.save(text);\n        log(saved)\n    }\n}\n\ntrait Notifier { fn send(&self, text: &str) -> bool; }\n";
    let outline = outline(source, Lang::Rust);
    let hub = outline.types.iter().find(|t| t.name == "Hub").unwrap();
    assert_eq!(hub.implements, ["Notifier"]);
    let send = &outline.functions[0];
    assert_eq!(send.function.name, "Hub::send");
    assert_eq!(send.owner.as_deref(), Some("Hub"));
    assert_eq!(send.calls[0].name, "save");
    assert_eq!(send.calls[0].bound.as_deref(), Some("saved"));
    assert!(
        outline
            .functions
            .iter()
            .any(|f| f.is_abstract && f.function.short == "send")
    );
}

#[test]
fn visibility_as_written_or_as_the_language_implies() {
    use Visibility::*;
    let java = outline(JAVA, Lang::Java);
    let of = |name: &str| {
        java.functions
            .iter()
            .find(|f| f.function.short == name && f.owner.as_deref() == Some("BugService"))
            .and_then(|f| f.visibility)
    };
    assert_eq!(of("create"), Some(Public));
    assert_eq!(of("save"), Some(Private));
    let users = java.variables.iter().find(|v| v.name == "users").unwrap();
    assert_eq!(users.visibility, Some(Private));
    let rust = outline(
        "pub fn a() {}\nfn b() {}\npub(crate) fn c() {}\n",
        Lang::Rust,
    );
    let seen: Vec<_> = rust.functions.iter().map(|f| f.visibility).collect();
    assert_eq!(seen, [Some(Public), Some(Private), Some(Internal)]);
    assert_eq!(visibility("", "_hidden", Lang::Python), Some(Private));
    assert_eq!(visibility("", "Serve", Lang::Go), Some(Public));
    assert_eq!(visibility("", "x", Lang::TypeScript), None);
}
