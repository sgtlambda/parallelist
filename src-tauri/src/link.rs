//! Following a link clicked in a terminal.
//!
//! A file link is previewed with Quick Look, never opened: what a terminal
//! prints is untrusted, and an OSC 8 link's text need not resemble its
//! target, so `open` on a `file://` URL would launch whatever app or script
//! it names. Web and mail links go to `open`, which hands them to the
//! default browser or mail client.

use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;

#[derive(Debug, PartialEq)]
pub enum Link {
    /// A local file, for Quick Look.
    Preview(PathBuf),
    /// A URL for the app that handles its scheme.
    Open(String),
}

/// What clicking `url` should do, or why it does nothing.
pub fn classify(url: &str) -> Result<Link, String> {
    let parsed = tauri::Url::parse(url).map_err(|e| format!("not a link: {e}"))?;
    match parsed.scheme() {
        "http" | "https" | "mailto" => Ok(Link::Open(parsed.into())),
        // Tools that follow the OSC 8 convention (`ls --hyperlink`, kitty's)
        // name the machine; Claude Code leaves the host empty. The parser
        // lowercases the host, and macOS hostnames usually aren't.
        "file" => match parsed.host_str() {
            None | Some("" | "localhost") => file_path(&parsed),
            Some(host) if hostname().is_some_and(|h| h.eq_ignore_ascii_case(host)) => {
                file_path(&parsed)
            }
            Some(host) => Err(format!("a file on another machine ({host})")),
        },
        scheme => Err(format!("links to {scheme}: aren't followed")),
    }
}

fn file_path(url: &tauri::Url) -> Result<Link, String> {
    // `to_file_path` refuses any host but this one, which a file URL keeps
    // as no host at all.
    let mut local = url.clone();
    let _ = local.set_host(Some("localhost"));
    local
        .to_file_path()
        .map(Link::Preview)
        .map_err(|_| format!("not a file path: {url}"))
}

fn hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer outlives the call and its length is passed with it.
    if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
        return None;
    }
    let end = buf.iter().position(|&b| b == 0)?;
    String::from_utf8(buf[..end].to_vec()).ok()
}

/// The Quick Look panel on screen, if any. One at a time: clicking the next
/// image replaces it rather than stacking panels.
static PREVIEW: Mutex<Option<Child>> = Mutex::new(None);

/// Preview or open `url`.
pub fn follow(url: &str) -> Result<(), String> {
    match classify(url)? {
        Link::Preview(path) => {
            if !path.exists() {
                return Err(format!("{} no longer exists", path.display()));
            }
            let mut preview = PREVIEW.lock().unwrap();
            if let Some(mut old) = preview.take() {
                let _ = old.kill();
                let _ = old.wait();
            }
            // `qlmanage -p` lives as long as its panel, so it is left running
            // rather than given a timeout, and reaped when replaced.
            let child = Command::new("/usr/bin/qlmanage")
                .arg("-p")
                .arg(&path)
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .map_err(|e| format!("could not start Quick Look: {e}"))?;
            *preview = Some(child);
            Ok(())
        }
        Link::Open(url) => {
            let out = crate::proc::run("/usr/bin/open", &[&url], None)?;
            if out.ok() {
                Ok(())
            } else {
                Err(format!("could not open {url}: {}", out.stderr.trim()))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn web_and_mail_links_are_opened() {
        for url in ["https://github.com/a/b/pull/1", "http://localhost:5173/", "mailto:a@b.c"] {
            assert_eq!(classify(url), Ok(Link::Open(url.to_string())));
        }
    }

    /// Claude Code links `[Image #1]` to the pasted image with an empty host.
    #[test]
    fn file_links_are_previewed() {
        let path = PathBuf::from("/Users/me/.claude/image cache/1.png");
        let expect = Ok(Link::Preview(path));
        assert_eq!(classify("file:///Users/me/.claude/image%20cache/1.png"), expect);
        assert_eq!(classify("file://localhost/Users/me/.claude/image%20cache/1.png"), expect);
        let named = format!("file://{}/Users/me/.claude/image%20cache/1.png", hostname().unwrap());
        assert_eq!(classify(&named), expect);
    }

    #[test]
    fn nothing_else_is_followed() {
        for url in [
            "file://elsewhere.example/etc/hosts",
            "x-apple.systempreferences:com.apple.preference.security",
            "javascript:alert(1)",
            "ssh://host",
            "not a url",
        ] {
            assert!(classify(url).is_err(), "{url}");
        }
    }
}
