//! Debugging real programs in each language with its real debugger.

use super::*;

/// Debugs `source` in a fresh project with a real debugger, found in
/// `JIG_DEBUGGERS_DIR`: run `command` under ⌃D with a breakpoint on
/// `line` (0-based), check `variable` shows `value`, resume, and check
/// the program printed `printed`.
#[allow(clippy::too_many_arguments)]
fn debug_project_live(
    cx: &mut TestAppContext,
    debugger: &'static str,
    files: &[(&str, &str)],
    source: &str,
    command: &str,
    line: u32,
    variable: &str,
    value: &str,
    printed: &str,
) {
    assert!(
        std::env::var_os("JIG_DEBUGGERS_DIR").is_some(),
        "JIG_DEBUGGERS_DIR: where the debuggers are installed"
    );
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join(".jig")).unwrap();
    for (name, text) in files {
        let path = root.join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    let source = root.join(source);
    std::fs::write(
        root.join(".jig/run.toml"),
        format!("[[run]]\nname = \"App\"\ncommand = \"{command}\"\n"),
    )
    .unwrap();
    let (window, workspace) = open(cx, &root);
    step(cx, window, move |_, cx| {
        jig_settings::update(cx, |s| s.debugging.set_enabled(debugger, true))
    });
    open_file(cx, window, &workspace, &source);
    let ws = workspace.clone();
    step(cx, window, move |window, cx| {
        window.render_frame(cx);
        ws.update(cx, |this, cx| {
            let text = this.editor().text(cx);
            let offset: usize = text
                .split_inclusive('\n')
                .take(line as usize)
                .map(str::len)
                .sum();
            this.editor()
                .state()
                .update(cx, |s, cx| s.set_selected_range(offset..offset, cx))
        });
        window.press("secondary-f8", cx);
    });
    step(cx, window, |window, cx| window.press("ctrl-d", cx));
    let wait = |cx: &mut TestAppContext, done: &dyn Fn(&Workspace, &gpui_kit::App) -> bool| {
        for _ in 0..600 {
            cx.executor()
                .advance_clock(std::time::Duration::from_millis(50));
            cx.run_until_parked();
            if cx.update(|cx| done(workspace.read(cx), cx)) {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let output = cx.update(|cx| workspace.read(cx).run_output(cx));
        panic!("timed out: {output:#?}");
    };
    wait(cx, &|this, cx| {
        this.run_picker_rows(cx)
            .is_some_and(|rows| rows.first().is_some_and(|row| row == "App"))
    });
    step(cx, window, |window, cx| window.press("enter", cx));
    let variable = variable.to_string();
    wait(cx, &|this, _| {
        this.debug_variable_rows()
            .iter()
            .any(|(_, name, _)| *name == variable)
    });
    cx.update(|cx| {
        let this = workspace.read(cx);
        assert_eq!(this.execution_line(cx), Some((source.clone(), line)));
        let rows = this.debug_variable_rows();
        eprintln!("{rows:#?}");
        // Some values carry an id that changes, as Java's `int[3]@8`.
        assert!(
            rows.iter().any(|(depth, name, shown)| {
                *depth == 1 && *name == variable && shown.starts_with(value)
            }),
            "{rows:#?}"
        );
    });
    step(cx, window, |window, cx| window.press("f9", cx));
    wait(cx, &|this, app| {
        this.run_output(app)
            .is_some_and(|(_, _, ending)| ending.is_some())
    });
    let (_, lines, _) = cx.update(|cx| workspace.read(cx).run_output(cx)).unwrap();
    let texts: Vec<String> = lines.iter().map(|line| line.text.to_string()).collect();
    eprintln!("{texts:#?}");
    assert!(texts.contains(&printed.to_string()), "{texts:#?}");
}

/// TypeScript on Node with js-debug, through its child session. Needs
/// Node 23.6+. `JIG_TS_COMMAND="npm start"` tries the way package.json
/// scripts run.
#[gpui_kit::test]
#[ignore]
fn debug_typescript_live(cx: &mut TestAppContext) {
    let command = std::env::var("JIG_TS_COMMAND").unwrap_or_else(|_| "node main.ts".into());
    debug_project_live(
        cx,
        "typescript",
        &[
            (
                "main.ts",
                "function total(numbers: number[]): number {\n  return numbers.reduce((a, b) => a + b, 0);\n}\nconst numbers: number[] = [1, 2, 3];\nconst sum: number = total(numbers);\nconsole.log(`total ${sum}`);\n",
            ),
            (
                "package.json",
                r#"{"name": "sample", "scripts": {"start": "node main.ts"}}"#,
            ),
        ],
        "main.ts",
        &command,
        1,
        "numbers",
        "(3) [1, 2, 3]",
        "total 6",
    );
}

/// Python with debugpy.
#[gpui_kit::test]
#[ignore]
fn debug_python_live(cx: &mut TestAppContext) {
    debug_project_live(
        cx,
        "python",
        &[(
            "main.py",
            "def total(numbers):\n    return sum(numbers)\n\nnumbers = [1, 2, 3]\nresult = total(numbers)\nprint(f\"total {result}\")\n",
        )],
        "main.py",
        "python3 main.py",
        1,
        "numbers",
        "[1, 2, 3]",
        "total 6",
    );
}

/// Java with java-debug in jdtls, which finds the main class and builds
/// the project itself. Needs a JDK, Maven, and `JIG_LIVE_LSP=1` so the
/// real jdtls runs; `JIG_CACHE_DIR` keeps its index elsewhere.
#[gpui_kit::test]
#[ignore]
fn debug_java_live(cx: &mut TestAppContext) {
    assert!(
        std::env::var_os("JIG_LIVE_LSP").is_some(),
        "JIG_LIVE_LSP=1: Java's debugger lives in jdtls"
    );
    debug_project_live(
        cx,
        "java",
        &[
            (
                "pom.xml",
                "<project xmlns=\"http://maven.apache.org/POM/4.0.0\">\n  <modelVersion>4.0.0</modelVersion>\n  <groupId>sample</groupId>\n  <artifactId>sample</artifactId>\n  <version>1.0</version>\n  <properties>\n    <maven.compiler.release>21</maven.compiler.release>\n  </properties>\n</project>\n",
            ),
            (
                "src/main/java/sample/Main.java",
                "package sample;\n\npublic class Main {\n    static int total(int[] numbers) {\n        int sum = 0;\n        for (int n : numbers) sum += n;\n        return sum;\n    }\n\n    public static void main(String[] args) {\n        int[] numbers = {1, 2, 3};\n        System.out.println(\"total \" + total(numbers));\n    }\n}\n",
            ),
        ],
        "src/main/java/sample/Main.java",
        "mvn -q compile exec:java -Dexec.mainClass=sample.Main",
        4,
        "numbers",
        "int[3]",
        "total 6",
    );
}

/// Go with Delve, which builds the program itself.
#[gpui_kit::test]
#[ignore]
fn debug_go_live(cx: &mut TestAppContext) {
    debug_project_live(
        cx,
        "go",
        &[
            (
                "main.go",
                "package main\n\nimport \"fmt\"\n\nfunc total(numbers []int) int {\n\tsum := 0\n\tfor _, n := range numbers {\n\t\tsum += n\n\t}\n\treturn sum\n}\n\nfunc main() {\n\tnumbers := []int{1, 2, 3}\n\tfmt.Printf(\"total %d\\n\", total(numbers))\n}\n",
            ),
            ("go.mod", "module sample\n\ngo 1.22\n"),
        ],
        "main.go",
        "go run .",
        6,
        "numbers",
        "[]int len: 3, cap: 3, [1,2,3]",
        "total 6",
    );
}
