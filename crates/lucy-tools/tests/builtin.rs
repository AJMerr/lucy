//! The built-in tools, called the way the core calls them: through an in-process MCP client.

use lucy_providers::testing::{FakeResponse, FakeServer};
use lucy_tools::builtin::{self, BuiltinOptions, PrivatePaths};
use rmcp::RoleClient;
use rmcp::model::CallToolRequestParams;
use rmcp::service::RunningService;
use serde_json::{Value, json};

struct Tools {
    dir: tempfile::TempDir,
    client: RunningService<RoleClient, ()>,
}

impl Tools {
    /// Built-in tools working in a fresh temp folder, which also stands in for home.
    async fn start() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let client = builtin::connect(BuiltinOptions {
            cwd: dir.path().to_owned(),
            private: PrivatePaths::defaults(Some(dir.path())),
        })
        .await
        .unwrap();
        Self { dir, client }
    }

    fn write(&self, rel: &str, content: impl AsRef<[u8]>) {
        let path = self.dir.path().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, content).unwrap();
    }

    fn read(&self, rel: &str) -> String {
        std::fs::read_to_string(self.dir.path().join(rel)).unwrap()
    }

    /// The tool's text output, and whether it reported an error.
    async fn call(&self, name: &'static str, arguments: Value) -> (String, bool) {
        let Value::Object(arguments) = arguments else {
            panic!("arguments must be an object")
        };
        let result = self
            .client
            .call_tool(CallToolRequestParams::new(name).with_arguments(arguments))
            .await
            .unwrap();
        let text = result
            .content
            .iter()
            .filter_map(|c| c.as_text().map(|t| t.text.clone()))
            .collect::<Vec<_>>()
            .join("\n");
        (text, result.is_error == Some(true))
    }

    async fn ok(&self, name: &'static str, arguments: Value) -> String {
        let (text, is_error) = self.call(name, arguments).await;
        assert!(!is_error, "{name} failed: {text}");
        text
    }

    async fn err(&self, name: &'static str, arguments: Value) -> String {
        let (text, is_error) = self.call(name, arguments).await;
        assert!(is_error, "{name} should have failed but said: {text}");
        text
    }
}

#[tokio::test]
async fn lists_every_tool_with_safety_labels() {
    let tools = Tools::start().await;

    let listed = tools.client.list_all_tools().await.unwrap();

    let mut names: Vec<&str> = listed.iter().map(|t| t.name.as_ref()).collect();
    names.sort();
    assert_eq!(
        names,
        [
            "edit_file",
            "fetch_url",
            "list_dir",
            "read_file",
            "run_command",
            "search_files",
            "write_file"
        ]
    );
    for tool in &listed {
        let hints = tool.annotations.as_ref().expect("every tool is labelled");
        let (read_only, open_world) = match tool.name.as_ref() {
            "read_file" | "list_dir" | "search_files" => (true, false),
            "fetch_url" => (true, true),
            "write_file" | "edit_file" => (false, false),
            "run_command" => (false, true),
            other => panic!("unexpected tool {other}"),
        };
        assert_eq!(hints.read_only_hint, Some(read_only), "{}", tool.name);
        assert_eq!(hints.open_world_hint, Some(open_world), "{}", tool.name);
        assert!(
            tool.description.as_deref().is_some_and(|d| d.len() > 20),
            "{} needs a description the model can use",
            tool.name
        );
    }
}

// --- read_file ---------------------------------------------------------------------------

#[tokio::test]
async fn read_file_resolves_paths_against_the_working_folder() {
    let tools = Tools::start().await;
    tools.write("notes/todo.txt", "buy milk\ncall Rebecca\n");

    assert_eq!(
        tools
            .ok("read_file", json!({"path": "notes/todo.txt"}))
            .await,
        "buy milk\ncall Rebecca\n"
    );
    let absolute = tools.dir.path().join("notes/todo.txt");
    assert_eq!(
        tools
            .ok("read_file", json!({"path": absolute.to_str().unwrap()}))
            .await,
        "buy milk\ncall Rebecca\n"
    );
}

#[tokio::test]
async fn read_file_pages_through_long_files() {
    let tools = Tools::start().await;
    tools.write("long.txt", "one\ntwo\nthree\nfour\n");

    let page = tools
        .ok(
            "read_file",
            json!({"path": "long.txt", "offset": 2, "limit": 2}),
        )
        .await;

    assert!(page.starts_with("two\nthree\n"), "{page}");
    assert!(
        page.contains("offset=4"),
        "should say how to continue: {page}"
    );
    assert!(!page.contains("four"), "{page}");
}

#[tokio::test]
async fn read_file_reports_missing_and_binary_files() {
    let tools = Tools::start().await;
    tools.write("data.bin", [0x7f, b'E', b'L', b'F', 0, 0, 0, 13]);

    let missing = tools.err("read_file", json!({"path": "nope.txt"})).await;
    assert!(missing.contains("nope.txt"), "{missing}");

    let binary = tools.err("read_file", json!({"path": "data.bin"})).await;
    assert!(binary.contains("binary"), "{binary}");
}

// --- list_dir ----------------------------------------------------------------------------

#[tokio::test]
async fn list_dir_shows_folders_first_with_a_slash() {
    let tools = Tools::start().await;
    tools.write("b.txt", "b");
    tools.write("a.txt", "a");
    tools.write("src/main.rs", "fn main() {}");

    let listing = tools.ok("list_dir", json!({})).await;

    let names: Vec<&str> = listing
        .lines()
        .map(|l| l.split_whitespace().next().unwrap())
        .collect();
    assert_eq!(names, ["src/", "a.txt", "b.txt"], "{listing}");
}

#[tokio::test]
async fn list_dir_of_a_missing_folder_is_an_error() {
    let tools = Tools::start().await;
    let err = tools.err("list_dir", json!({"path": "ghost"})).await;
    assert!(err.contains("ghost"), "{err}");
}

// --- search_files ------------------------------------------------------------------------

#[tokio::test]
async fn search_files_finds_matching_lines_with_paths_and_line_numbers() {
    let tools = Tools::start().await;
    tools.write("src/main.rs", "fn main() {\n    wake_up();\n}\n");
    tools.write("src/lib.rs", "pub fn wake_up() {}\n");
    tools.write("README.md", "nothing to see\n");

    let found = tools
        .ok("search_files", json!({"pattern": "wake_up"}))
        .await;

    let mut lines: Vec<String> = found.lines().map(|l| l.replace('\\', "/")).collect();
    lines.sort();
    assert_eq!(
        lines,
        [
            "src/lib.rs:1: pub fn wake_up() {}",
            "src/main.rs:2:     wake_up();"
        ]
    );
}

#[tokio::test]
async fn search_files_can_filter_by_glob() {
    let tools = Tools::start().await;
    tools.write("a.rs", "needle\n");
    tools.write("b.md", "needle\n");

    let found = tools
        .ok("search_files", json!({"pattern": "needle", "glob": "*.md"}))
        .await;

    assert_eq!(found.trim(), "b.md:1: needle");
}

#[tokio::test]
async fn search_files_never_looks_inside_private_or_ignored_files() {
    let tools = Tools::start().await;
    tools.write("notes.txt", "API_KEY lives elsewhere\n");
    tools.write(".env", "API_KEY=sk-live-123\n");
    tools.write("certs/server.key", "API_KEY in a key file\n");
    tools.write(".ssh/config", "API_KEY in ssh config\n");
    tools.write(".gitignore", "build/\n");
    tools.write("build/out.txt", "API_KEY in build output\n");

    let found = tools
        .ok("search_files", json!({"pattern": "API_KEY"}))
        .await;

    assert_eq!(found.trim(), "notes.txt:1: API_KEY lives elsewhere");
}

#[tokio::test]
async fn search_files_cant_reach_private_files_by_another_route() {
    let tools = Tools::start().await;
    tools.write(".ssh/config", "API_KEY in ssh config\n");
    tools.write(".ssh/notes/more", "API_KEY deeper\n");

    let dotted = tools
        .ok(
            "search_files",
            json!({"pattern": "API_KEY", "path": "./.ssh/./notes/.."}),
        )
        .await;

    assert_eq!(dotted.trim(), "No matches.");
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(tools.dir.path().join(".ssh"), tools.dir.path().join("keys"))
            .unwrap();
        let linked = tools
            .ok(
                "search_files",
                json!({"pattern": "API_KEY", "path": "keys"}),
            )
            .await;
        assert_eq!(linked.trim(), "No matches.");
    }
}

#[tokio::test]
async fn search_files_rejects_a_bad_pattern() {
    let tools = Tools::start().await;
    let err = tools
        .err("search_files", json!({"pattern": "(unclosed"}))
        .await;
    assert!(err.contains("pattern"), "{err}");
}

// --- write_file and edit_file ------------------------------------------------------------

#[tokio::test]
async fn write_file_creates_folders_and_overwrites() {
    let tools = Tools::start().await;

    tools
        .ok(
            "write_file",
            json!({"path": "out/plan.md", "content": "# Plan\n"}),
        )
        .await;
    assert_eq!(tools.read("out/plan.md"), "# Plan\n");

    tools
        .ok(
            "write_file",
            json!({"path": "out/plan.md", "content": "# Plan B\n"}),
        )
        .await;
    assert_eq!(tools.read("out/plan.md"), "# Plan B\n");
}

#[tokio::test]
async fn edit_file_replaces_exactly_one_match() {
    let tools = Tools::start().await;
    tools.write("main.rs", "let a = 1;\nlet b = 2;\n");

    tools
        .ok(
            "edit_file",
            json!({"path": "main.rs", "old_text": "let b = 2;", "new_text": "let b = 3;"}),
        )
        .await;

    assert_eq!(tools.read("main.rs"), "let a = 1;\nlet b = 3;\n");
}

#[tokio::test]
async fn edit_file_refuses_missing_or_ambiguous_text() {
    let tools = Tools::start().await;
    tools.write("main.rs", "x = 1\nx = 1\n");

    let missing = tools
        .err(
            "edit_file",
            json!({"path": "main.rs", "old_text": "y = 1", "new_text": "y = 2"}),
        )
        .await;
    assert!(missing.contains("not found"), "{missing}");

    let ambiguous = tools
        .err(
            "edit_file",
            json!({"path": "main.rs", "old_text": "x = 1", "new_text": "x = 2"}),
        )
        .await;
    assert!(
        ambiguous.contains('2') && ambiguous.contains("replace_all"),
        "{ambiguous}"
    );
    assert_eq!(
        tools.read("main.rs"),
        "x = 1\nx = 1\n",
        "file must be untouched"
    );

    tools
        .ok(
            "edit_file",
            json!({"path": "main.rs", "old_text": "x = 1", "new_text": "x = 2", "replace_all": true}),
        )
        .await;
    assert_eq!(tools.read("main.rs"), "x = 2\nx = 2\n");
}

// --- run_command -------------------------------------------------------------------------

#[tokio::test]
async fn run_command_runs_in_the_working_folder_and_reports_output() {
    let tools = Tools::start().await;
    tools.write("marker.txt", "here");

    let listing_command = if cfg!(windows) { "dir /b" } else { "ls" };
    let output = tools
        .ok("run_command", json!({"command": listing_command}))
        .await;

    assert!(output.contains("marker.txt"), "{output}");
}

#[tokio::test]
async fn run_command_failure_includes_exit_code_and_stderr() {
    let tools = Tools::start().await;

    let output = tools
        .err(
            "run_command",
            json!({"command": "echo oops 1>&2 && exit 3"}),
        )
        .await;

    assert!(output.contains("exit code 3"), "{output}");
    assert!(output.contains("oops"), "{output}");
}

#[tokio::test]
async fn run_command_times_out() {
    let tools = Tools::start().await;
    let slow = if cfg!(windows) {
        "ping -n 30 127.0.0.1"
    } else {
        "sleep 30"
    };

    let started = std::time::Instant::now();
    let output = tools
        .err("run_command", json!({"command": slow, "timeout_secs": 1}))
        .await;

    assert!(output.contains("timed out"), "{output}");
    assert!(
        started.elapsed().as_secs() < 10,
        "the command wasn't stopped"
    );
}

// --- fetch_url ---------------------------------------------------------------------------

#[tokio::test]
async fn fetch_url_returns_readable_text_from_html() {
    let page = "<html><head><title>Night City</title><style>p{color:red}</style>\
                <script>alert('x')</script></head>\
                <body><h1>Afterlife</h1><p>Drinks &amp; legends.</p></body></html>";
    let server = FakeServer::start([FakeResponse::body(200, "text/html", page)]).await;
    let tools = Tools::start().await;

    let text = tools
        .ok("fetch_url", json!({"url": format!("{}/bar", server.url())}))
        .await;

    assert!(text.contains("Afterlife"), "{text}");
    assert!(text.contains("Drinks & legends."), "{text}");
    for leftover in ["<p>", "color:red", "alert"] {
        assert!(!text.contains(leftover), "{leftover} leaked: {text}");
    }
}

#[tokio::test]
async fn fetch_url_reports_http_errors() {
    let server = FakeServer::start([FakeResponse::body(404, "text/plain", "gone")]).await;
    let tools = Tools::start().await;

    let err = tools
        .err(
            "fetch_url",
            json!({"url": format!("{}/missing", server.url())}),
        )
        .await;

    assert!(err.contains("404"), "{err}");
}
