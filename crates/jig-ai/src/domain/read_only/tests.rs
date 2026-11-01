use super::*;

fn project() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(dir.path().join("src/bookings")).unwrap();
    dir
}

#[test]
fn commands_that_read_the_project_run_without_asking() {
    let project = project();
    for command in [
        "ls -la src",
        "rg -n \"fn main\" crates | head -20",
        "cd src && git ls-files | wc -l",
        "sed -n 1,80p src/main.rs",
        "sed -n '10,20p;30p' src/main.rs",
        "sed 's/a/b/g' src/main.rs | head",
        "sed -n '/fn main/,/^}/p' src/main.rs",
        "find . -name '*.rs' | xargs grep -l Booking",
        "ls src/bookings 2>/dev/null",
        "find . -name '*.svelte' 2> /dev/null | head -50",
        "cat package.json >/dev/null 2>&1",
        "rg -n 'book|pay' src &>/dev/null",
        "grep -E \"Booking|Payment\" -r src | sort",
        "rg -n '->' src",
        "grep -rn \"/api/bookings\" src",
        "grep -n 'end$' src/main.rs",
        "grep -ril \"medlem\\|member\" --include=*.java src | head -40",
        "git branch -a",
        "git log --oneline -20",
        "awk -F: '{print $1}' src/main.rs",
        "cd src && ls ../src",
        "echo \"---\"",
        "ls -la && ls .jig/map/sketches/ 2>/dev/null",
        "find backend/src/hold backend/src/bookings -type f | sort",
        "rg -il \"medlem|member\" --glob '!node_modules' --glob '!.jig' . | head -40",
        "grep -rni \"rabat\\|discount\" backend frontend --exclude-dir=node_modules 2>/dev/null | head -20; echo \"---\"; grep -n \"getHold\\|getArrangementer\" frontend/src/api/api.ts | head",
    ] {
        assert!(command_only_reads(command, project.path()), "{command}");
    }
}

#[test]
fn commands_that_change_or_leave_the_project_ask() {
    let project = project();
    for command in [
        "rm -rf target",
        "echo hi > notes.txt",
        "sed -i s/a/b/ src/main.rs",
        "sed -i.bak s/a/b/ src/main.rs",
        "sed -n '1,5w out.txt' src/main.rs",
        "sed 's/a/b/w out.txt' src/main.rs",
        "sed 's/a/date/e' src/main.rs",
        "find . -name '*.tmp' -delete",
        "find . -fprint out.txt",
        "git checkout main",
        "git branch -D old",
        "git branch feature",
        "git diff --output=patch.diff",
        "cat $(which python)",
        "npm install",
        "ls > files.txt 2>/dev/null",
        "cat a >> /dev/nullish",
        "grep x src 2>errors.log",
        "echo \"$(rm -rf src)\"",
        "diff <(rm -rf src) b",
        "ls; rm -rf src 2>/dev/null",
        "sort -o out.txt src/main.rs",
        "uniq in.txt out.txt",
        "rg --pre ./run.sh x",
        "awk '{print > \"out\"}' src/main.rs",
        "awk 'BEGIN{system(\"rm x\")}'",
        "cat ~/.ssh/id_rsa",
        "cat $HOME/.ssh/id_rsa",
        "ls ..",
        "cd .. && ls",
    ] {
        assert!(!command_only_reads(command, project.path()), "{command}");
    }
    // Folders only Unix has: elsewhere they don't exist, so read nothing.
    #[cfg(unix)]
    for command in ["cat /etc/hosts", "grep -r token /etc/*"] {
        assert!(!command_only_reads(command, project.path()), "{command}");
    }
}
