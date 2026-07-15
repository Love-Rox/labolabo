//! `labolabo`: the control-protocol CLI client (`docs/control-protocol.md`
//! §8) -- the `labolabo tab open --title reviewer -- claude ...` flagship
//! use case (`plans/012-task-model-and-control-cli.md` §2) is this binary.
//!
//! Deliberately a *separate* small binary target in the `labolabo-app`
//! package (not a `labolabo-core::src/bin` bin like `labolabo-hook`,
//! `labolabo-core/src/bin/labolabo-hook.rs`): `labolabo-hook` is invoked by
//! Claude Code itself on every hook event and has no reason to know
//! anything about Tasks/tabs; `labolabo` is the control surface for this
//! app's own Task/tab model, so it belongs next to `app.rs`'s
//! `LaboLaboApp::dispatch_control` (the code it's a client of) even though
//! that means this binary target pulls in the package's `gpui` dependency
//! at *build* time (it doesn't use gpui itself -- the linker doesn't pull
//! in unreferenced code, so the produced binary itself isn't gpui-bloated;
//! see the PR description for this trade-off).
//!
//! All the actual protocol logic (request/response (de)serialization,
//! socket-path resolution, the `--task current`/ambient-context rules) is
//! `labolabo_core::control_protocol`/`labolabo_core::control` -- this file
//! is intentionally thin: hand-rolled argv parsing (no `clap`/similar --
//! docs/control-protocol.md §8 explains why: 4 subcommands, at most 3 flags
//! each, well under the complexity `clap`'s auto-usage/completion features
//! earn their weight at), building one `ControlRequest`, sending it, and
//! printing the response per docs/control-protocol.md §8's output/exit-code
//! rules.

use std::collections::HashMap;
use std::process::ExitCode;

use labolabo_core::{
    parse_response, resolve_socket_path, resolve_task_flag, send_control_request, ControlRequest,
};

const USAGE: &str = "\
Usage:
  labolabo [--socket <path>] tab open [--task <id|current>] [--title <t>] [--json] [-- <command...>]
  labolabo [--socket <path>] task list [--json]
  labolabo [--socket <path>] tab list [--task <id|current>] [--all] [--json]
  labolabo [--socket <path>] focus --task <id> [--json]
  labolabo [--socket <path>] focus --pane <id> [--json]";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => code,
        Err(message) => {
            eprintln!("error: {message}");
            eprintln!("{USAGE}");
            // Usage/parse errors never reach the server -- bucketed with
            // "connection failure" (docs/control-protocol.md §8's exit-code
            // table only defines 3 codes).
            ExitCode::from(2)
        }
    }
}

// MARK: - argv -> ParsedArgs

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Subcommand {
    TabOpen,
    TaskList,
    TabList,
    Focus,
}

#[derive(Debug, Default)]
struct ParsedArgs {
    socket: Option<String>,
    json: bool,
    task: Option<String>,
    title: Option<String>,
    all: bool,
    pane: Option<String>,
    command_argv: Option<Vec<String>>,
}

/// Splits `args` at the first bare `--` token: everything after it is
/// `tab_open`'s trailing command argv (docs/control-protocol.md §5.1),
/// everything before it is flags/subcommand words to scan normally. `None`
/// second element means no `--` was present.
fn split_trailing_command(args: &[String]) -> (&[String], Option<&[String]>) {
    match args.iter().position(|a| a == "--") {
        Some(index) => (&args[..index], Some(&args[index + 1..])),
        None => (args, None),
    }
}

fn parse_args(args: &[String]) -> Result<(Subcommand, ParsedArgs), String> {
    let (head, tail) = split_trailing_command(args);

    let mut positionals = Vec::new();
    let mut parsed = ParsedArgs::default();
    let mut i = 0;
    while i < head.len() {
        let arg = head[i].as_str();
        macro_rules! next_value {
            ($flag:expr) => {{
                i += 1;
                head.get(i)
                    .ok_or_else(|| format!("{} requires a value", $flag))?
                    .clone()
            }};
        }
        match arg {
            "--socket" => parsed.socket = Some(next_value!("--socket")),
            "--json" => parsed.json = true,
            "--task" => parsed.task = Some(next_value!("--task")),
            "--title" => parsed.title = Some(next_value!("--title")),
            "--all" => parsed.all = true,
            "--pane" => parsed.pane = Some(next_value!("--pane")),
            other if other.starts_with("--") => {
                return Err(format!("unknown flag: {other}"));
            }
            other => positionals.push(other.to_string()),
        }
        i += 1;
    }

    let subcommand = match positionals.as_slice() {
        [a, b] if a == "tab" && b == "open" => Subcommand::TabOpen,
        [a, b] if a == "task" && b == "list" => Subcommand::TaskList,
        [a, b] if a == "tab" && b == "list" => Subcommand::TabList,
        [a] if a == "focus" => Subcommand::Focus,
        [] => return Err("missing subcommand".to_string()),
        _ => return Err(format!("unknown subcommand: {}", positionals.join(" "))),
    };

    if let Some(tail) = tail {
        if subcommand != Subcommand::TabOpen {
            return Err("-- <command...> is only valid for `tab open`".to_string());
        }
        parsed.command_argv = Some(tail.to_vec());
    }

    match subcommand {
        Subcommand::TabOpen => {
            if parsed.all || parsed.pane.is_some() {
                return Err("--all/--pane are not valid for `tab open`".to_string());
            }
        }
        Subcommand::TaskList => {
            if parsed.task.is_some()
                || parsed.title.is_some()
                || parsed.all
                || parsed.pane.is_some()
            {
                return Err("`task list` takes no --task/--title/--all/--pane flags".to_string());
            }
        }
        Subcommand::TabList => {
            if parsed.title.is_some() || parsed.pane.is_some() {
                return Err("--title/--pane are not valid for `tab list`".to_string());
            }
        }
        Subcommand::Focus => {
            if parsed.title.is_some() || parsed.all || parsed.command_argv.is_some() {
                return Err("--title/--all/-- <command...> are not valid for `focus`".to_string());
            }
            match (&parsed.task, &parsed.pane) {
                (Some(_), Some(_)) => {
                    return Err("focus: specify exactly one of --task/--pane, not both".to_string())
                }
                (None, None) => return Err("focus: --task or --pane is required".to_string()),
                _ => {}
            }
        }
    }

    Ok((subcommand, parsed))
}

/// Builds the `ControlRequest` for `subcommand`/`parsed` (docs/control-
/// protocol.md §5/§6). `--task`'s client-side "current"/omitted collapsing
/// (docs/control-protocol.md §4.2) applies to `tab_open`/`tab_list` via
/// [`resolve_task_flag`]; `focus`'s `--task`/`--pane` are passed through
/// literally (§5.4).
fn build_request(subcommand: Subcommand, parsed: &ParsedArgs) -> ControlRequest {
    match subcommand {
        Subcommand::TabOpen => ControlRequest::new(
            "tab_open",
            serde_json::json!({
                "task": resolve_task_flag(parsed.task.as_deref()),
                "title": parsed.title,
                "command": parsed.command_argv,
            }),
        ),
        Subcommand::TaskList => ControlRequest::new("task_list", serde_json::json!({})),
        Subcommand::TabList => ControlRequest::new(
            "tab_list",
            serde_json::json!({
                "task": resolve_task_flag(parsed.task.as_deref()),
                "all": parsed.all,
            }),
        ),
        Subcommand::Focus => ControlRequest::new(
            "focus",
            serde_json::json!({
                "task": parsed.task,
                "pane": parsed.pane,
            }),
        ),
    }
}

// MARK: - run

fn run(args: &[String]) -> Result<ExitCode, String> {
    let (subcommand, parsed) = parse_args(args)?;
    let request = build_request(subcommand, &parsed);

    let env: HashMap<String, String> = std::env::vars().collect();
    let request = request.with_ambient_context(&env);

    let socket_path = match resolve_socket_path(parsed.socket.as_deref(), &env) {
        Ok(path) => path,
        Err(err) => {
            eprintln!("error: {err}");
            return Ok(ExitCode::from(2));
        }
    };

    let response_bytes = match send_control_request(&socket_path, &request.to_bytes()) {
        Ok(bytes) => bytes,
        Err(err) => {
            eprintln!("error: could not reach labolabo at {socket_path}: {err}");
            return Ok(ExitCode::from(2));
        }
    };

    if parsed.json {
        println!("{}", String::from_utf8_lossy(&response_bytes));
    }

    let response = match parse_response(&response_bytes) {
        Ok(response) => response,
        Err(err) => {
            if !parsed.json {
                eprintln!("error: {err}");
            }
            return Ok(ExitCode::from(1));
        }
    };

    if response.ok {
        if !parsed.json {
            print_human_success(subcommand, response.result.as_ref());
        }
        Ok(ExitCode::SUCCESS)
    } else {
        if !parsed.json {
            eprintln!(
                "error: {}",
                response.error.as_deref().unwrap_or("unknown error")
            );
        }
        Ok(ExitCode::from(1))
    }
}

// MARK: - human-readable output (docs/control-protocol.md §8)

fn print_human_success(subcommand: Subcommand, result: Option<&serde_json::Value>) {
    let Some(result) = result else {
        println!("ok");
        return;
    };
    let str_field = |key: &str| result.get(key).and_then(|v| v.as_str());

    match subcommand {
        Subcommand::TabOpen => {
            let pane_id = str_field("pane_id").unwrap_or("?");
            let task_id = str_field("task_id").unwrap_or("?");
            println!("opened pane {pane_id} in task {task_id}");
        }
        Subcommand::TaskList => {
            let tasks = result.get("tasks").and_then(|v| v.as_array());
            let Some(tasks) = tasks else {
                println!("(no tasks)");
                return;
            };
            if tasks.is_empty() {
                println!("(no tasks)");
            }
            for task in tasks {
                let id = task.get("id").and_then(|v| v.as_str()).unwrap_or("?");
                let title = task.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                let kind = task.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                let repo = task
                    .get("repo_name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("?");
                println!("{id}  [{kind}]  {title}  ({repo})");
            }
        }
        Subcommand::TabList => {
            let tabs = result.get("tabs").and_then(|v| v.as_array());
            let Some(tabs) = tabs else {
                println!("(no tabs)");
                return;
            };
            if tabs.is_empty() {
                println!("(no tabs)");
            }
            for tab in tabs {
                let task_id = tab.get("task_id").and_then(|v| v.as_str()).unwrap_or("?");
                let pane_id = tab.get("pane_id").and_then(|v| v.as_str()).unwrap_or("-");
                let title = tab.get("title").and_then(|v| v.as_str()).unwrap_or("?");
                let kind = tab.get("kind").and_then(|v| v.as_str()).unwrap_or("?");
                let focused = tab
                    .get("focused")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let marker = if focused { "*" } else { " " };
                println!("{marker} {task_id}  {pane_id}  [{kind}]  {title}");
            }
        }
        Subcommand::Focus => {
            let task_id = str_field("task_id").unwrap_or("?");
            match str_field("pane_id") {
                Some(pane_id) => println!("focused pane {pane_id} in task {task_id}"),
                None => println!("focused task {task_id}"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // MARK: - split_trailing_command

    #[test]
    fn split_trailing_command_no_dashdash() {
        let args = vec!["tab".to_string(), "open".to_string()];
        let (head, tail) = split_trailing_command(&args);
        assert_eq!(head, &args[..]);
        assert_eq!(tail, None);
    }

    #[test]
    fn split_trailing_command_splits_at_dashdash() {
        let args = vec![
            "tab".to_string(),
            "open".to_string(),
            "--".to_string(),
            "claude".to_string(),
            "-p".to_string(),
        ];
        let (head, tail) = split_trailing_command(&args);
        assert_eq!(head, &["tab".to_string(), "open".to_string()]);
        assert_eq!(tail, Some(&["claude".to_string(), "-p".to_string()][..]));
    }

    // MARK: - parse_args

    fn args(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn parses_tab_open_with_all_flags_and_trailing_command() {
        let (sub, parsed) = parse_args(&args(&[
            "--socket",
            "/tmp/s.sock",
            "tab",
            "open",
            "--task",
            "t1",
            "--title",
            "reviewer",
            "--json",
            "--",
            "claude",
            "-p",
        ]))
        .unwrap();
        assert_eq!(sub, Subcommand::TabOpen);
        assert_eq!(parsed.socket.as_deref(), Some("/tmp/s.sock"));
        assert_eq!(parsed.task.as_deref(), Some("t1"));
        assert_eq!(parsed.title.as_deref(), Some("reviewer"));
        assert!(parsed.json);
        assert_eq!(
            parsed.command_argv,
            Some(vec!["claude".to_string(), "-p".to_string()])
        );
    }

    #[test]
    fn parses_tab_open_with_no_flags() {
        let (sub, parsed) = parse_args(&args(&["tab", "open"])).unwrap();
        assert_eq!(sub, Subcommand::TabOpen);
        assert_eq!(parsed.task, None);
        assert_eq!(parsed.command_argv, None);
    }

    #[test]
    fn parses_task_list() {
        let (sub, _parsed) = parse_args(&args(&["task", "list", "--json"])).unwrap();
        assert_eq!(sub, Subcommand::TaskList);
    }

    #[test]
    fn parses_tab_list_with_all() {
        let (sub, parsed) = parse_args(&args(&["tab", "list", "--all"])).unwrap();
        assert_eq!(sub, Subcommand::TabList);
        assert!(parsed.all);
    }

    #[test]
    fn parses_focus_with_task() {
        let (sub, parsed) = parse_args(&args(&["focus", "--task", "t1"])).unwrap();
        assert_eq!(sub, Subcommand::Focus);
        assert_eq!(parsed.task.as_deref(), Some("t1"));
    }

    #[test]
    fn parses_focus_with_pane() {
        let (sub, parsed) = parse_args(&args(&["focus", "--pane", "p1"])).unwrap();
        assert_eq!(sub, Subcommand::Focus);
        assert_eq!(parsed.pane.as_deref(), Some("p1"));
    }

    #[test]
    fn focus_requires_exactly_one_of_task_or_pane() {
        assert!(parse_args(&args(&["focus"])).is_err());
        assert!(parse_args(&args(&["focus", "--task", "t1", "--pane", "p1"])).is_err());
    }

    #[test]
    fn missing_subcommand_is_an_error() {
        assert!(parse_args(&args(&["--json"])).is_err());
    }

    #[test]
    fn unknown_subcommand_is_an_error() {
        assert!(parse_args(&args(&["bogus"])).is_err());
    }

    #[test]
    fn unknown_flag_is_an_error() {
        assert!(parse_args(&args(&["task", "list", "--bogus"])).is_err());
    }

    #[test]
    fn trailing_command_only_valid_for_tab_open() {
        assert!(parse_args(&args(&["task", "list", "--", "echo", "hi"])).is_err());
    }

    #[test]
    fn tab_open_rejects_all_and_pane_flags() {
        assert!(parse_args(&args(&["tab", "open", "--all"])).is_err());
        assert!(parse_args(&args(&["tab", "open", "--pane", "p1"])).is_err());
    }

    #[test]
    fn task_list_rejects_unrelated_flags() {
        assert!(parse_args(&args(&["task", "list", "--task", "t1"])).is_err());
    }

    #[test]
    fn tab_list_rejects_title_and_pane() {
        assert!(parse_args(&args(&["tab", "list", "--title", "x"])).is_err());
        assert!(parse_args(&args(&["tab", "list", "--pane", "p1"])).is_err());
    }

    #[test]
    fn flag_missing_its_value_is_an_error() {
        assert!(parse_args(&args(&["tab", "open", "--task"])).is_err());
    }

    // MARK: - build_request

    #[test]
    fn build_request_tab_open_resolves_current_task_to_null() {
        let (sub, parsed) = parse_args(&args(&["tab", "open", "--task", "current"])).unwrap();
        let request = build_request(sub, &parsed);
        assert_eq!(request.command, "tab_open");
        assert_eq!(request.params["task"], serde_json::Value::Null);
    }

    #[test]
    fn build_request_tab_open_keeps_an_explicit_task_id() {
        let (sub, parsed) = parse_args(&args(&["tab", "open", "--task", "t1"])).unwrap();
        let request = build_request(sub, &parsed);
        assert_eq!(request.params["task"], "t1");
    }

    #[test]
    fn build_request_focus_keeps_task_literal_even_when_current() {
        // focus never applies resolve_task_flag's "current" collapsing
        // (docs/control-protocol.md §5.4).
        let (sub, parsed) = parse_args(&args(&["focus", "--task", "current"])).unwrap();
        let request = build_request(sub, &parsed);
        assert_eq!(request.params["task"], "current");
    }

    #[test]
    fn build_request_task_list_has_empty_params() {
        let (sub, parsed) = parse_args(&args(&["task", "list"])).unwrap();
        let request = build_request(sub, &parsed);
        assert_eq!(request.params, serde_json::json!({}));
    }
}
