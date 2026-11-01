/// A small project: Java in two packages, TypeScript in two folders.
pub fn fixture() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let files = [
        (
            "server/src/org/demo/bugs/BugService.java",
            "package org.demo.bugs;\n\nimport org.demo.users.UserService;\nimport org.slf4j.Logger;\n\npublic class BugService {\n    private final UserService users;\n    private final Logger log;\n    private final Notifier notifier;\n\n    public Bug create(String title, long userId) {\n        User user = users.find(userId);\n        Bug bug = save(title, user);\n        log.info(\"made\");\n        notifier.send(bug);\n        return bug;\n    }\n\n    private Bug save(String title, User user) {\n        return new Bug(title, user);\n    }\n}\n",
        ),
        (
            "server/src/org/demo/bugs/Notifier.java",
            "package org.demo.bugs;\n\npublic interface Notifier {\n    void send(Bug bug);\n}\n",
        ),
        (
            "server/src/org/demo/bugs/MailNotifier.java",
            "package org.demo.bugs;\n\npublic class MailNotifier implements Notifier {\n    public void send(Bug bug) {\n        format(bug);\n    }\n\n    private String format(Bug bug) { return \"\"; }\n}\n",
        ),
        (
            "server/src/org/demo/users/UserService.java",
            "package org.demo.users;\n\npublic class UserService {\n    public User find(long id) {\n        return lookup(id);\n    }\n\n    private User lookup(long id) { return null; }\n}\n",
        ),
        (
            "client/src/rooms.ts",
            "import { connect } from './net/socket'\nimport * as mediasoup from 'mediasoup'\n\nexport async function join(id: string, done: () => void): Promise<void> {\n    const socket = connect(id)\n    await mediasoup.createRouter()\n    send(socket, id)\n    done()\n}\n\nfunction send(socket: Socket, id: string) {}\n",
        ),
        (
            "client/src/net/socket.ts",
            "export function connect(url: string): Socket {\n    return open(url)\n}\n\nfunction open(url: string): Socket { return null as any }\n",
        ),
        (
            "client/node_modules/mediasoup/index.ts",
            "export function createRouter() {}\n",
        ),
    ];
    for (name, text) in files {
        let path = dir.path().join(name);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    dir
}
