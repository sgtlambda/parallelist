//! What Claude Code last asked, read from its own session transcript.
//!
//! Its status sequences (OSC 7501) say *that* a turn ended, never what it
//! ended on: `done` carries no message, even when the turn closes on a
//! question (observed driving a real `claude` 2.1.309 in a pty). The
//! transcript has it. Claude Code records each running process in
//! `~/.claude/sessions/<pid>.json` — its session id and the directory it
//! started in — and appends the session to
//! `~/.claude/projects/<that directory, mangled>/<session id>.jsonl`. The
//! terminal knows the pid: it leads the foreground process group. So the
//! transcript is found exactly, even with several sessions in one
//! directory, where "the newest file" would pick the wrong one.
//!
//! All of this is Claude Code's private layout, not an interface. Anything
//! missing or unreadable is `None`, and the card simply shows no question.

use serde_json::Value;
use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

/// How much of the end of a transcript is searched for the last message.
/// Tool results can be large, but the last assistant entry sits near the end.
const TAIL: u64 = 1024 * 1024;

/// Longest text handed to the card, which shows a line of it.
const LIMIT: usize = 400;

fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("CLAUDE_CONFIG_DIR") {
        return Some(PathBuf::from(dir));
    }
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".claude"))
}

/// Claude Code's directory name for a project: every character that isn't
/// an ASCII letter or digit becomes `-`.
fn mangle(dir: &str) -> String {
    dir.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

/// The transcript of the Claude Code process `pid`, if that is one.
fn transcript(config: &Path, pid: i32) -> Option<PathBuf> {
    let session: Value =
        serde_json::from_slice(&std::fs::read(config.join("sessions").join(format!("{pid}.json"))).ok()?)
            .ok()?;
    let id = session.get("sessionId")?.as_str()?;
    let file = format!("{id}.jsonl");
    let projects = config.join("projects");
    if let Some(cwd) = session.get("cwd").and_then(Value::as_str) {
        let direct = projects.join(mangle(cwd)).join(&file);
        if direct.exists() {
            return Some(direct);
        }
    }
    // A path too long for the mangling above is shortened some other way.
    std::fs::read_dir(&projects)
        .ok()?
        .flatten()
        .map(|entry| entry.path().join(&file))
        .find(|path| path.exists())
}

/// The last assistant entry in the transcript at `path`.
fn last_assistant(path: &Path) -> Option<Value> {
    let mut file = File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    file.seek(SeekFrom::Start(len.saturating_sub(TAIL))).ok()?;
    let mut tail = Vec::new();
    file.read_to_end(&mut tail).ok()?;
    let tail = String::from_utf8_lossy(&tail);
    tail.lines()
        .rev()
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .find(|entry| entry.get("type").and_then(Value::as_str) == Some("assistant"))
}

/// What an assistant entry leaves the user to answer: the question it asks
/// through `AskUserQuestion`, what it wants permission for, or the end of
/// what it said.
fn ask(entry: &Value) -> Option<String> {
    let content = entry.get("message")?.get("content")?.as_array()?;
    let text: Vec<&str> = content
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text")?.as_str())
        .collect();
    if !text.is_empty() {
        return closing(&text.join("\n\n"));
    }
    let tool = content
        .iter()
        .rev()
        .find(|block| block.get("type").and_then(Value::as_str) == Some("tool_use"))?;
    let input = tool.get("input")?;
    if tool.get("name").and_then(Value::as_str) == Some("AskUserQuestion") {
        let question = input.get("questions")?.get(0)?.get("question")?.as_str()?;
        return Some(clip(question));
    }
    // Waiting on permission, most likely: say for what.
    let what = ["description", "command", "file_path", "url", "pattern"]
        .iter()
        .find_map(|key| input.get(*key)?.as_str())?;
    let name = tool.get("name").and_then(Value::as_str).unwrap_or("a tool");
    Some(clip(&format!("{name}: {what}")))
}

/// The part of a message the user is left with: its last question if it
/// has one, else its last paragraph.
fn closing(text: &str) -> Option<String> {
    let paragraphs: Vec<&str> = text
        .split("\n\n")
        .map(str::trim)
        .filter(|p| !p.is_empty() && !p.starts_with("```"))
        .collect();
    let last = *paragraphs.last()?;
    let question = paragraphs.iter().rev().take(3).find_map(|p| last_question(p));
    Some(clip(&plain(question.unwrap_or(last))))
}

/// The last sentence of `paragraph` that ends in a question mark.
fn last_question(paragraph: &str) -> Option<&str> {
    let end = paragraph.rfind('?')? + 1;
    let head = &paragraph[..end];
    let start = head[..head.len() - 1]
        .rfind(['.', '!', '?', '\n', ':'])
        .map_or(0, |i| i + 1);
    let sentence = head[start..].trim();
    (sentence.len() > 3).then_some(sentence)
}

/// Markdown's markup, dropped: the card shows plain text.
fn plain(text: &str) -> String {
    text.lines()
        .map(|line| line.trim_start_matches(['#', '>', '-', '*', ' ']))
        .collect::<Vec<_>>()
        .join(" ")
        .replace("**", "")
        .replace('`', "")
}

fn clip(text: &str) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    match text.char_indices().nth(LIMIT) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text,
    }
}

/// What the Claude Code running as `pid` last asked of the user, if it is
/// Claude Code and that can be read.
pub fn question(pid: i32) -> Option<String> {
    let path = transcript(&config_dir()?, pid)?;
    ask(&last_assistant(&path)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn mangles_a_directory_as_claude_code_does() {
        assert_eq!(
            mangle("/Users/me/Code/apps/parallelist-hold-tab"),
            "-Users-me-Code-apps-parallelist-hold-tab"
        );
        assert_eq!(mangle("/tmp/a.b_c"), "-tmp-a-b-c");
    }

    #[test]
    fn a_message_ends_on_its_last_question() {
        let text = "I looked at the header.\n\nIt breaks on narrow screens. Should I fix the \
                    padding too, or only the overflow?\n\nEither works.";
        assert_eq!(
            closing(text).as_deref(),
            Some("Should I fix the padding too, or only the overflow?")
        );
        assert_eq!(
            closing("Done. All **tests** pass with `cargo test`.").as_deref(),
            Some("Done. All tests pass with cargo test.")
        );
    }

    #[test]
    fn a_tool_call_says_what_it_waits_for() {
        let asking = json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "name": "AskUserQuestion",
             "input": {"questions": [{"question": "Which name?"}]}}]}});
        assert_eq!(ask(&asking).as_deref(), Some("Which name?"));
        let running = json!({"type": "assistant", "message": {"content": [
            {"type": "tool_use", "name": "Bash",
             "input": {"command": "rm -rf build", "description": "Remove the build"}}]}});
        assert_eq!(ask(&running).as_deref(), Some("Bash: Remove the build"));
    }

    #[test]
    fn finds_the_transcript_of_a_process() {
        let config = std::env::temp_dir().join(format!("parallelist-cc-{}", std::process::id()));
        std::fs::remove_dir_all(&config).ok();
        let project = config.join("projects").join(mangle("/work/app"));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(config.join("sessions")).unwrap();
        std::fs::write(
            config.join("sessions/42.json"),
            r#"{"pid":42,"sessionId":"abc","cwd":"/work/app"}"#,
        )
        .unwrap();
        let lines = [
            json!({"type": "user", "message": {"content": "hi"}}),
            json!({"type": "assistant", "message": {"content": [
                {"type": "text", "text": "What's your favourite colour?"}]}}),
            json!({"type": "system"}),
        ];
        let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(project.join("abc.jsonl"), body).unwrap();

        let path = transcript(&config, 42).expect("found");
        assert_eq!(
            ask(&last_assistant(&path).unwrap()).as_deref(),
            Some("What's your favourite colour?")
        );
        assert!(transcript(&config, 43).is_none());
        std::fs::remove_dir_all(&config).ok();
    }
}
