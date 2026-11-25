//! Searching what the terminals show, for the command palette.
//!
//! Nothing is indexed in the background. When the palette asks, each
//! terminal's screen is rebuilt from its backlog (`Terminals::screens`, a few
//! milliseconds for a full one), cut into chunks of a few lines, and scored
//! against the query two ways:
//!
//! - **Literally**: the share of the query's words found on the screen. This
//!   is what finds identifiers, issue numbers and error codes, which a small
//!   embedding model blurs.
//! - **By meaning**: cosine similarity between the query and each chunk,
//!   embedded by a model that runs on this machine (`MODELS`, through
//!   fastembed and ONNX Runtime). A terminal scores as its best chunk.
//!
//! Embeddings are kept per chunk text, so asking again re-embeds only what
//! changed on screen. Chunks end where a line's hash says so, not every n
//! lines (`chunks`): with fixed windows a dev server printing one line would
//! shift every window and make every chunk new.
//!
//! **The model runs in a worker process**: this same binary, started with
//! `WORKER_FLAG` (`main.rs`), answering one line of JSON per request. ONNX
//! Runtime takes about 280 MB plus twice the model file once a model is
//! loaded, and dropping the session gives only half of that back, so a
//! model loaded in the app itself would cost hundreds of MB for good. The
//! worker starts on the first search (or when the setting is turned on or
//! the model changed, which downloads it), and is stopped after `IDLE`
//! without a search, when the system gets all of it back. It is also where
//! a crash in native code would land, rather than in the process that
//! holds every terminal. Like every child process (invariant 1), it has its
//! own process group and a timeout on everything asked of it.
//!
//! Until the model is ready, searches are literal only.
//!
//! Only models whose embeddings don't depend on what else is in the batch
//! are offered. A dynamically quantized model rescales each batch, so a
//! query embedded on its own would not be comparable with chunks embedded
//! earlier; fastembed refuses to split their input for the same reason.

use fastembed::{EmbeddingModel, TextEmbedding, TextInitOptions};
use serde::{Deserialize, Serialize};
use std::collections::hash_map::DefaultHasher;
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Write};
use std::os::unix::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};

/// A model the user can choose in Settings.
pub struct Model {
    pub id: &'static str,
    pub label: &'static str,
    /// Which model it is, for the Settings dropdown.
    pub name: &'static str,
    /// Roughly what is downloaded, once, in MB.
    pub download_mb: u32,
    /// Roughly what the worker takes while it runs, in MB.
    pub memory_mb: u32,
    embedding: EmbeddingModel,
    /// Cosine similarity at or below which a chunk counts as unrelated to a
    /// query, and at or above which it is as related as it gets. Measured
    /// with `cargo run --release --example screen_search`: the floor sits
    /// above what nine in ten unrelated screens score, the ceiling a little
    /// above the median of the right ones.
    floor: f32,
    ceiling: f32,
}

/// From fastest to best, all from the sentence-transformers family, which
/// spreads unrelated and related text far apart (0.1 against 0.4 and up).
/// Retrieval-tuned models (BGE, Arctic) ranked about as well on the same
/// screens but crowd everything into 0.45..0.75, where no threshold tells a
/// match from noise. None is big: the largest is a "base"-sized model.
pub const MODELS: &[Model] = &[
    Model {
        id: "minilm-l6",
        label: "Small",
        name: "all-MiniLM-L6-v2",
        download_mb: 90,
        memory_mb: 500,
        embedding: EmbeddingModel::AllMiniLML6V2,
        floor: 0.20,
        ceiling: 0.50,
    },
    Model {
        id: "minilm-l12",
        label: "Medium",
        name: "all-MiniLM-L12-v2",
        download_mb: 135,
        memory_mb: 550,
        embedding: EmbeddingModel::AllMiniLML12V2,
        floor: 0.20,
        ceiling: 0.50,
    },
    Model {
        id: "mpnet-base",
        label: "Large",
        name: "all-mpnet-base-v2",
        download_mb: 440,
        memory_mb: 1200,
        embedding: EmbeddingModel::AllMpnetBaseV2,
        floor: 0.22,
        ceiling: 0.50,
    },
];

pub const DEFAULT_MODEL: &str = "minilm-l6";

/// The first argument that makes this binary a worker (`main.rs`).
pub const WORKER_FLAG: &str = "--screen-search-worker";

pub fn model(id: &str) -> Option<&'static Model> {
    MODELS.iter().find(|m| m.id == id)
}

impl Model {
    pub fn floor_and_ceiling(&self) -> (f32, f32) {
        (self.floor, self.ceiling)
    }

    fn options(&self, cache_dir: PathBuf) -> TextInitOptions {
        TextInitOptions::new(self.embedding.clone())
            .with_cache_dir(cache_dir)
            .with_max_length(MAX_TOKENS)
            .with_show_download_progress(false)
    }
}

/// A model as Settings lists it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: &'static str,
    pub label: &'static str,
    pub name: &'static str,
    pub download_mb: u32,
    pub memory_mb: u32,
}

pub fn models() -> Vec<ModelInfo> {
    MODELS
        .iter()
        .map(|m| ModelInfo {
            id: m.id,
            label: m.label,
            name: m.name,
            download_mb: m.download_mb,
            memory_mb: m.memory_mb,
        })
        .collect()
}

/// Where the model is.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase", tag = "state")]
pub enum ModelStatus {
    /// Screen search is turned off.
    Off,
    /// Turned on, with no worker running: none has been asked for since
    /// launch, or it was stopped for being idle.
    Idle { model: String },
    /// The worker is starting: downloading the model, the first time, or
    /// reading it from disk.
    Loading { model: String },
    Ready { model: String },
    Failed { model: String, error: String },
}

/// A terminal whose screen matches the query.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hit {
    pub path: String,
    /// 0..1, the better of how many of the query's words are on the screen
    /// and how close in meaning its best chunk is.
    pub relevance: f32,
    /// The text that matched: the line with the query's words, or the
    /// chunk closest in meaning.
    pub snippet: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Found {
    pub query: String,
    pub status: ModelStatus,
    pub hits: Vec<Hit>,
}

/// One terminal's screen, as text.
pub struct Screen {
    pub path: String,
    pub text: String,
}

/// A failed worker is started again on a search after this long, not on
/// every keystroke: it is usually the network, on the first download.
const RETRY_AFTER: Duration = Duration::from_secs(60);

/// The worker is stopped after this long without a search. Long enough
/// that opening the palette again a minute later finds it running.
const IDLE: Duration = Duration::from_secs(180);

/// How long a worker may take to start: the first time, that includes
/// downloading the model.
const START_TIMEOUT: Duration = Duration::from_secs(600);

/// How long one batch of chunks may take. A full screen on the large model
/// takes a few seconds.
const EMBED_TIMEOUT: Duration = Duration::from_secs(60);

/// Queries shorter than this are left to the branch names.
pub const MIN_QUERY: usize = 3;

/// Below this a hit isn't worth a row.
const MIN_RELEVANCE: f32 = 0.05;

const SNIPPET_CHARS: usize = 200;

/// Chunks embedded per run of the model.
const EMBED_BATCH: usize = 8;

/// Tokens a chunk is cut to. A chunk of `CHUNK_CHARS` is well within it;
/// the models' own limit (512 for some) only makes a rare long run of
/// symbols costlier.
const MAX_TOKENS: usize = 256;

struct Inner {
    /// The model chosen in Settings while screen search is on.
    wanted: Option<&'static Model>,
    status: ModelStatus,
    worker: Option<Arc<Mutex<Worker>>>,
    failed_at: Option<Instant>,
    last_used: Instant,
    /// Bumped on every change of model, so a worker that finishes starting
    /// after the setting moved on is thrown away.
    generation: u64,
}

pub type OnStatus = Box<dyn Fn(&ModelStatus) + Send + Sync>;

/// How a worker is started: the app's own binary, or a stand-in in tests.
type Spawn = Box<dyn Fn(&Model) -> Command + Send + Sync>;

pub struct ScreenSearch {
    inner: Mutex<Inner>,
    /// Embeddings by chunk text, for the model in `inner`. Holds only what
    /// the last search saw, and outlives the worker, so one started again
    /// after being idle embeds only what changed meanwhile.
    embeddings: Mutex<HashMap<u64, Arc<[f32]>>>,
    on_status: OnStatus,
    spawn: Spawn,
}

impl ScreenSearch {
    /// Workers are this binary, started with `WORKER_FLAG`, keeping models
    /// in `cache_dir`.
    pub fn new(cache_dir: PathBuf, on_status: OnStatus) -> Arc<Self> {
        Self::with_spawn(
            on_status,
            Box::new(move |model: &Model| {
                let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("parallelist"));
                let mut cmd = Command::new(exe);
                cmd.arg(WORKER_FLAG).arg(model.id).arg(&cache_dir);
                cmd
            }),
        )
    }

    fn with_spawn(on_status: OnStatus, spawn: Spawn) -> Arc<Self> {
        let search = Arc::new(Self {
            inner: Mutex::new(Inner {
                wanted: None,
                status: ModelStatus::Off,
                worker: None,
                failed_at: None,
                last_used: Instant::now(),
                generation: 0,
            }),
            embeddings: Mutex::default(),
            on_status,
            spawn,
        });
        let weak = Arc::downgrade(&search);
        std::thread::spawn(move || stop_when_idle(weak));
        search
    }

    pub fn status(&self) -> ModelStatus {
        self.inner.lock().unwrap().status.clone()
    }

    fn set_status(&self, inner: &mut Inner, status: ModelStatus) {
        if inner.status != status {
            inner.status = status;
            (self.on_status)(&inner.status);
        }
    }

    /// Follow the settings: `None` when screen search is off. A model that
    /// changes is dropped, and the new one started at once if `start` (the
    /// user just chose it, so its download begins), else on the first
    /// search.
    pub fn configure(self: &Arc<Self>, model_id: Option<&str>, start: bool) {
        let wanted = model_id.map(|id| model(id).unwrap_or(&MODELS[0]));
        let mut inner = self.inner.lock().unwrap();
        if inner.wanted.map(|m| m.id) != wanted.map(|m| m.id) {
            inner.generation += 1;
            inner.wanted = wanted;
            inner.worker = None;
            inner.failed_at = None;
            self.embeddings.lock().unwrap().clear();
            let status = match wanted {
                Some(m) => ModelStatus::Idle { model: m.id.into() },
                None => ModelStatus::Off,
            };
            self.set_status(&mut inner, status);
        }
        if start {
            inner.last_used = Instant::now();
            self.start_worker(&mut inner);
        }
    }

    /// Start a worker for the wanted model on a thread of its own, unless
    /// one runs or is starting, or one failed a moment ago.
    fn start_worker(self: &Arc<Self>, inner: &mut Inner) {
        let Some(model) = inner.wanted else { return };
        match inner.status {
            ModelStatus::Loading { .. } | ModelStatus::Ready { .. } | ModelStatus::Off => return,
            ModelStatus::Failed { .. }
                if inner.failed_at.is_some_and(|at| at.elapsed() < RETRY_AFTER) =>
            {
                return
            }
            _ => {}
        }
        let generation = inner.generation;
        self.set_status(inner, ModelStatus::Loading { model: model.id.into() });
        let this = Arc::clone(self);
        std::thread::spawn(move || {
            let started = Worker::start((this.spawn)(model));
            let mut inner = this.inner.lock().unwrap();
            if inner.generation != generation {
                return;
            }
            let status = match started {
                Ok(worker) => {
                    inner.worker = Some(Arc::new(Mutex::new(worker)));
                    inner.last_used = Instant::now();
                    ModelStatus::Ready { model: model.id.into() }
                }
                Err(error) => {
                    inner.failed_at = Some(Instant::now());
                    ModelStatus::Failed { model: model.id.into(), error }
                }
            };
            this.set_status(&mut inner, status);
        });
    }

    /// A worker that failed mid-search is gone; the next search after
    /// `RETRY_AFTER` starts another.
    fn worker_failed(&self, generation: u64, error: String) {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation != generation {
            return;
        }
        inner.worker = None;
        inner.failed_at = Some(Instant::now());
        if let Some(model) = inner.wanted {
            self.set_status(&mut inner, ModelStatus::Failed { model: model.id.into(), error });
        }
    }

    /// Score every screen against `query`. Literal matching always runs;
    /// matching by meaning once the worker is ready, which this starts if
    /// it isn't.
    pub fn search(self: &Arc<Self>, query: &str, screens: &[Screen]) -> Found {
        let (status, worker, model, generation) = {
            let mut inner = self.inner.lock().unwrap();
            inner.last_used = Instant::now();
            self.start_worker(&mut inner);
            (inner.status.clone(), inner.worker.clone(), inner.wanted, inner.generation)
        };
        let query = query.trim();
        let mut hits = Vec::new();
        if status != ModelStatus::Off && query.chars().count() >= MIN_QUERY {
            let words = query_words(query);
            let prepared: Vec<Prepared> = screens.iter().map(prepare).collect();
            let meaning = match (worker, model) {
                (Some(worker), Some(model)) => {
                    match self.by_meaning(&worker, model, query, &prepared) {
                        Ok(meaning) => meaning,
                        Err(error) => {
                            self.worker_failed(generation, error);
                            Vec::new()
                        }
                    }
                }
                _ => Vec::new(),
            };
            for (i, screen) in prepared.iter().enumerate() {
                let (share, line) = literal(&words, &screen.lines);
                let (closeness, chunk) = meaning.get(i).copied().unwrap_or((0.0, None));
                let (relevance, snippet) = if share >= closeness {
                    (share, line.map(|l| screen.lines[l].clone()))
                } else {
                    (closeness, chunk.map(|c| screen.chunks[c].replace('\n', " · ")))
                };
                if relevance >= MIN_RELEVANCE {
                    hits.push(Hit {
                        path: screen.path.clone(),
                        relevance,
                        snippet: clip(&snippet.unwrap_or_default(), SNIPPET_CHARS),
                    });
                }
            }
            hits.sort_by(|a, b| b.relevance.total_cmp(&a.relevance));
        }
        Found { query: query.to_string(), status: self.status(), hits }
    }

    /// For each screen, how close in meaning its best chunk is to the
    /// query (0..1, after the model's floor and ceiling) and which chunk
    /// that is. Embeds only chunks not seen in the last search.
    fn by_meaning(
        &self,
        worker: &Mutex<Worker>,
        model: &Model,
        query: &str,
        screens: &[Prepared],
    ) -> Result<Vec<(f32, Option<usize>)>, String> {
        let mut known = std::mem::take(&mut *self.embeddings.lock().unwrap());
        let mut missing: Vec<(u64, &str)> = Vec::new();
        for screen in screens {
            for chunk in &screen.chunks {
                let key = hash(chunk);
                if !known.contains_key(&key) && !missing.iter().any(|(k, _)| *k == key) {
                    missing.push((key, chunk));
                }
            }
        }
        let mut texts: Vec<&str> = missing.iter().map(|(_, text)| *text).collect();
        texts.push(query);
        let mut vectors = worker.lock().unwrap().embed(&texts)?;
        let query = normalized(vectors.pop().ok_or("no query embedding")?);
        for ((key, _), vector) in missing.iter().zip(vectors) {
            known.insert(*key, normalized(vector));
        }

        let mut kept = HashMap::new();
        let scores = screens
            .iter()
            .map(|screen| {
                let mut best = (f32::MIN, None);
                for (c, chunk) in screen.chunks.iter().enumerate() {
                    let key = hash(chunk);
                    let Some(vector) = known.get(&key) else { continue };
                    let sim = dot(&query, vector);
                    if sim > best.0 {
                        best = (sim, Some(c));
                    }
                    kept.insert(key, Arc::clone(vector));
                }
                let scaled =
                    ((best.0 - model.floor) / (model.ceiling - model.floor)).clamp(0.0, 1.0);
                (scaled, best.1.filter(|_| scaled > 0.0))
            })
            .collect();
        *self.embeddings.lock().unwrap() = kept;
        Ok(scores)
    }
}

/// Stop the worker once nothing has searched for `IDLE`. Runs for as long
/// as the `ScreenSearch` does.
fn stop_when_idle(search: Weak<ScreenSearch>) {
    loop {
        std::thread::sleep(Duration::from_secs(15));
        let Some(search) = search.upgrade() else { return };
        let mut inner = search.inner.lock().unwrap();
        if inner.worker.is_some() && inner.last_used.elapsed() >= IDLE {
            let worker = inner.worker.take();
            if let Some(model) = inner.wanted {
                search.set_status(&mut inner, ModelStatus::Idle { model: model.id.into() });
            }
            drop(inner);
            // Waits for a search still using it, then kills it.
            drop(worker);
        }
    }
}

/// The worker process, from the app's side.
struct Worker {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

/// What a worker answers, one line each.
#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum Reply {
    Ready,
    Vectors(Vec<Vec<f32>>),
    Error(String),
}

impl Worker {
    /// Start `cmd` and wait until it says its model is loaded.
    fn start(mut cmd: Command) -> Result<Self, String> {
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        let mut child = cmd.spawn().map_err(|e| format!("could not start the model: {e}"))?;
        let stdin = child.stdin.take().ok_or("no stdin")?;
        let stdout = child.stdout.take().ok_or("no stdout")?;
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        let mut worker = Worker { child, stdin, lines };
        match worker.reply(START_TIMEOUT)? {
            Reply::Ready => Ok(worker),
            Reply::Error(e) => Err(e),
            Reply::Vectors(_) => Err("the model answered before it was asked".into()),
        }
    }

    fn reply(&mut self, timeout: Duration) -> Result<Reply, String> {
        let line = self.lines.recv_timeout(timeout).map_err(|e| match e {
            RecvTimeoutError::Timeout => "the model stopped responding".to_string(),
            RecvTimeoutError::Disconnected => "the model stopped".to_string(),
        })?;
        serde_json::from_str(&line).map_err(|e| format!("the model answered nonsense: {e}"))
    }

    /// One vector per text, in order.
    fn embed(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        let mut request = serde_json::to_string(texts).map_err(|e| e.to_string())?;
        request.push('\n');
        self.stdin
            .write_all(request.as_bytes())
            .and_then(|()| self.stdin.flush())
            .map_err(|_| "the model stopped".to_string())?;
        match self.reply(EMBED_TIMEOUT)? {
            Reply::Vectors(vectors) if vectors.len() == texts.len() => Ok(vectors),
            Reply::Vectors(_) => Err("the model answered for the wrong number of texts".into()),
            Reply::Error(e) => Err(e),
            Reply::Ready => Err("the model answered out of turn".into()),
        }
    }
}

impl Drop for Worker {
    fn drop(&mut self) {
        // The whole group, as for every child: a worker is one process,
        // but nothing it might start should outlive it.
        // SAFETY: killpg on a group we created; a failure is not actionable.
        unsafe { libc::killpg(self.child.id() as i32, libc::SIGKILL) };
        let _ = self.child.wait();
    }
}

/// The worker's side: load the model, say so, then answer each line of
/// texts on stdin with their vectors, until stdin closes. Returns the exit
/// code. Started by `main.rs` when the first argument is `WORKER_FLAG`.
pub fn serve(model_id: &str, cache_dir: PathBuf) -> i32 {
    let mut out = std::io::stdout().lock();
    let mut send = |reply: Reply| {
        serde_json::to_writer(&mut out, &reply).is_ok() && writeln!(out).is_ok() && out.flush().is_ok()
    };
    let Some(model) = model(model_id) else {
        send(Reply::Error(format!("unknown model {model_id}")));
        return 2;
    };
    let mut embedder = match TextEmbedding::try_new(model.options(cache_dir)) {
        Ok(embedder) => embedder,
        Err(e) => {
            send(Reply::Error(format!("could not load {}: {e}", model.name)));
            return 1;
        }
    };
    if !send(Reply::Ready) {
        return 1;
    }
    for line in std::io::stdin().lines().map_while(Result::ok) {
        let reply = match serde_json::from_str::<Vec<String>>(&line) {
            Ok(texts) => match embedder.embed(&texts, Some(EMBED_BATCH)) {
                Ok(vectors) => Reply::Vectors(vectors),
                Err(e) => Reply::Error(e.to_string()),
            },
            Err(e) => Reply::Error(format!("not a request: {e}")),
        };
        if !send(reply) {
            return 1;
        }
    }
    0
}

/// When this process was started as a worker, be one and return its exit
/// code.
pub fn serve_from_args() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != WORKER_FLAG {
        return None;
    }
    let model = args.next()?.to_string_lossy().to_string();
    let cache_dir = PathBuf::from(args.next()?);
    Some(serve(&model, cache_dir))
}

/// A screen made ready to score.
struct Prepared {
    path: String,
    lines: Vec<String>,
    chunks: Vec<String>,
}

fn prepare(screen: &Screen) -> Prepared {
    let lines = clean_lines(&screen.text);
    let chunks = chunks(&lines);
    Prepared { path: screen.path.clone(), lines, chunks }
}

/// Box drawing, block elements, geometric shapes, braille spinners, and the
/// dingbats and technical symbols programs draw as bullets and spinners
/// (`⏺`, `⎿`, `✻`, `❯`). They carry no meaning and would only pull every
/// TUI's chunks towards each other.
fn decoration(c: char) -> bool {
    matches!(c as u32,
        0x2500..=0x25FF | 0x2800..=0x28FF | 0x2700..=0x27BF | 0x23BF | 0x23FA)
}

/// Longest a line may be before it is cut. Models stop at a few hundred
/// tokens anyway, and a minified bundle printed in one line says nothing.
const LINE_CHARS: usize = 300;

/// The screen's lines that say something: decoration gone, whitespace
/// collapsed, and lines with fewer than three letters or digits dropped
/// (prompts, rules, a lone spinner).
fn clean_lines(text: &str) -> Vec<String> {
    text.lines()
        .filter_map(|line| {
            let line: String = line.chars().map(|c| if decoration(c) { ' ' } else { c }).collect();
            let line = line.split_whitespace().collect::<Vec<_>>().join(" ");
            (line.chars().filter(|c| c.is_alphanumeric()).count() >= 3)
                .then(|| clip(&line, LINE_CHARS))
        })
        .collect()
}

const CHUNK_LINES: usize = 6;
const CHUNK_CHARS: usize = 480;

/// Consecutive lines, grouped a few at a time. A chunk ends after a line
/// whose hash is divisible by 4, or when it is full, so where chunks end
/// depends on the lines themselves rather than their position: once output
/// scrolls the screen, the chunks in the middle are the same as before and
/// keep their embeddings.
fn chunks(lines: &[String]) -> Vec<String> {
    let mut out = Vec::new();
    let mut current: Vec<&str> = Vec::new();
    let mut len = 0;
    for line in lines {
        if !current.is_empty() && len + line.len() > CHUNK_CHARS {
            out.push(current.join("\n"));
            current.clear();
            len = 0;
        }
        current.push(line);
        len += line.len() + 1;
        if current.len() >= CHUNK_LINES || hash(line).is_multiple_of(4) {
            out.push(current.join("\n"));
            current.clear();
            len = 0;
        }
    }
    if !current.is_empty() {
        out.push(current.join("\n"));
    }
    out
}

/// Words too common to say anything when found on a screen. Shorter ones
/// are skipped anyway: "on" is in every `On branch …`.
const STOPWORDS: &[&str] = &[
    "the", "and", "for", "with", "from", "that", "this", "are", "was", "but", "not", "you", "all",
    "can", "has", "have", "into", "its", "our", "out", "what", "when", "where", "which", "who",
    "why", "how",
];

/// The query's words worth looking for literally, lowercased.
fn query_words(query: &str) -> Vec<String> {
    query
        .split_whitespace()
        .map(str::to_lowercase)
        .filter(|w| w.chars().count() >= 3 && !STOPWORDS.contains(&w.as_str()))
        .collect()
}

/// The share of `words` found on the screen, and the line holding most of
/// them, the lowest on the screen among equals (the newest).
fn literal(words: &[String], lines: &[String]) -> (f32, Option<usize>) {
    if words.is_empty() {
        return (0.0, None);
    }
    let lower: Vec<String> = lines.iter().map(|l| l.to_lowercase()).collect();
    let found = words.iter().filter(|w| lower.iter().any(|l| l.contains(w.as_str()))).count();
    let best = lower
        .iter()
        .enumerate()
        .map(|(i, l)| (words.iter().filter(|w| l.contains(w.as_str())).count(), i))
        .filter(|(n, _)| *n > 0)
        .max()
        .map(|(_, i)| i);
    (found as f32 / words.len() as f32, best)
}

fn hash(text: &str) -> u64 {
    let mut hasher = DefaultHasher::new();
    text.hash(&mut hasher);
    hasher.finish()
}

fn normalized(mut v: Vec<f32>) -> Arc<[f32]> {
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    }
    v.into()
}

fn dot(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b).map(|(x, y)| x * y).sum()
}

fn clip(text: &str, limit: usize) -> String {
    match text.char_indices().nth(limit) {
        Some((i, _)) => format!("{}…", &text[..i]),
        None => text.to_string(),
    }
}

/// For calibrating a model's floor and ceiling (`examples/screen_search.rs`):
/// each query's best cosine similarity to each screen, before scaling. Runs
/// the model in this process.
pub fn similarities(
    model_id: &str,
    cache_dir: PathBuf,
    queries: &[&str],
    screens: &[Screen],
) -> Result<Vec<Vec<f32>>, String> {
    let model = model(model_id).ok_or("unknown model")?;
    let mut embedder = TextEmbedding::try_new(model.options(cache_dir)).map_err(|e| e.to_string())?;
    let mut embed = |texts: &[String]| -> Result<Vec<Arc<[f32]>>, String> {
        Ok(embedder
            .embed(texts, Some(EMBED_BATCH))
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(normalized)
            .collect())
    };
    let chunk_vectors: Vec<Vec<Arc<[f32]>>> =
        screens.iter().map(|s| embed(&prepare(s).chunks)).collect::<Result<_, _>>()?;
    let query_vectors = embed(&queries.iter().map(|q| q.to_string()).collect::<Vec<_>>())?;
    Ok(query_vectors
        .iter()
        .map(|q| {
            chunk_vectors
                .iter()
                .map(|vs| vs.iter().map(|v| dot(q, v)).fold(f32::MIN, f32::max))
                .collect()
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decoration_and_empty_lines_go() {
        let text = "╭──────╮\n│ ✻ Welcome │\n╰──────╯\n\n⏺ Read(src/main.rs)\n  ⎿  Read 42 lines\n> \n❯ \n";
        assert_eq!(clean_lines(text), ["Welcome", "Read(src/main.rs)", "Read 42 lines"]);
    }

    #[test]
    fn a_scrolled_screen_keeps_its_middle_chunks() {
        let lines: Vec<String> =
            (0..60).map(|i| format!("GET /api/orders/{i} 200 in {}ms", i * 7 % 50)).collect();
        let before = chunks(&lines[0..40]);
        let after = chunks(&lines[3..43]);
        let kept = after.iter().filter(|c| before.contains(c)).count();
        assert!(kept + 3 >= after.len(), "only the edges change: {kept} of {}", after.len());
        assert!(before.iter().all(|c| c.lines().count() <= CHUNK_LINES));
    }

    #[test]
    fn literal_matches_count_words_and_point_at_the_newest_line() {
        let lines: Vec<String> = ["stripe webhook failed", "retrying", "Stripe webhook ok", "done"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let words = query_words("Stripe WEBHOOK refund");
        let (share, line) = literal(&words, &lines);
        assert!((share - 2.0 / 3.0).abs() < 1e-6);
        assert_eq!(line, Some(2));
        assert_eq!(literal(&query_words("on it"), &lines), (0.0, None), "too short to say anything");
        assert_eq!(query_words("what the header does on mobile"), ["header", "does", "mobile"]);
    }

    fn screens() -> Vec<Screen> {
        vec![
            Screen {
                path: "/a".into(),
                text: "$ cargo test\ntest result: FAILED. 41 passed; 1 failed\n".into(),
            },
            Screen { path: "/b".into(), text: "VITE ready in 412 ms\nhmr update Header.vue\n".into() },
        ]
    }

    /// A worker that answers like the real one, from a shell: each text
    /// gets [1, 0] if it mentions "vite", else [0, 1].
    fn fake_worker() -> Command {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(
            r#"echo '"ready"'
            while read -r line; do
              out=""
              rest="$line"
              while [ -n "$rest" ]; do
                item="${rest%%\",\"*}"
                case "$item" in *[Vv][Ii][Tt][Ee]*) v="[1,0]";; *) v="[0,1]";; esac
                out="$out${out:+,}$v"
                [ "$item" = "$rest" ] && break
                rest="${rest#*\",\"}"
              done
              echo "{\"vectors\":[$out]}"
            done"#,
        );
        cmd
    }

    fn wait_for(search: &ScreenSearch, want: impl Fn(&ModelStatus) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while !want(&search.status()) {
            assert!(Instant::now() < deadline, "status stayed {:?}", search.status());
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn off_finds_nothing_and_short_queries_are_left_to_branch_names() {
        let search = ScreenSearch::with_spawn(Box::new(|_| {}), Box::new(|_| fake_worker()));
        assert!(search.search("failed", &screens()).hits.is_empty());
        search.configure(Some("minilm-l6"), false);
        assert!(search.search("te", &screens()).hits.is_empty());
    }

    #[test]
    fn until_the_worker_is_ready_the_search_is_literal() {
        let search = ScreenSearch::with_spawn(
            Box::new(|_| {}),
            Box::new(|_| {
                let mut cmd = Command::new("/bin/sh");
                cmd.args(["-c", "sleep 5"]);
                cmd
            }),
        );
        search.configure(Some("minilm-l6"), false);
        let found = search.search("test failed", &screens());
        assert_eq!(found.status, ModelStatus::Loading { model: "minilm-l6".into() });
        assert_eq!(found.hits.len(), 1);
        assert_eq!(found.hits[0].path, "/a");
        assert_eq!(found.hits[0].relevance, 1.0);
        assert_eq!(found.hits[0].snippet, "test result: FAILED. 41 passed; 1 failed");
        search.configure(None, false);
    }

    #[test]
    fn a_ready_worker_matches_by_meaning() {
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&statuses);
        let search = ScreenSearch::with_spawn(
            Box::new(move |s| seen.lock().unwrap().push(s.clone())),
            Box::new(|_| fake_worker()),
        );
        search.configure(Some("minilm-l6"), true);
        wait_for(&search, |s| matches!(s, ModelStatus::Ready { .. }));
        // Half the words are on /b's screen; its meaning matches fully.
        let found = search.search("Vite thing", &screens());
        let b = found.hits.iter().find(|h| h.path == "/b").expect("/b by meaning");
        assert_eq!(b.relevance, 1.0);
        assert!(b.snippet.contains("VITE ready"), "{}", b.snippet);
        assert!(found.hits.iter().all(|h| h.path != "/a"), "{:?}", found.hits);
        assert_eq!(
            *statuses.lock().unwrap(),
            [
                ModelStatus::Idle { model: "minilm-l6".into() },
                ModelStatus::Loading { model: "minilm-l6".into() },
                ModelStatus::Ready { model: "minilm-l6".into() },
            ]
        );
        // Turning it off stops the worker.
        search.configure(None, false);
        assert_eq!(search.status(), ModelStatus::Off);
    }

    #[test]
    fn a_worker_that_dies_fails_and_the_search_goes_on_literally() {
        let search = ScreenSearch::with_spawn(
            Box::new(|_| {}),
            Box::new(|_| {
                let mut cmd = Command::new("/bin/sh");
                cmd.args(["-c", "echo '\"ready\"'; read -r line; exit 3"]);
                cmd
            }),
        );
        search.configure(Some("minilm-l6"), true);
        wait_for(&search, |s| matches!(s, ModelStatus::Ready { .. }));
        let found = search.search("hmr update", &screens());
        assert!(matches!(found.status, ModelStatus::Failed { .. }), "{:?}", found.status);
        assert_eq!(found.hits.len(), 1, "literal still: {:?}", found.hits);
        // Not started again on the next keystroke.
        assert!(matches!(search.search("hmr updates", &screens()).status, ModelStatus::Failed { .. }));
    }

    #[test]
    fn a_worker_that_cannot_load_says_why() {
        let search = ScreenSearch::with_spawn(
            Box::new(|_| {}),
            Box::new(|_| {
                let mut cmd = Command::new("/bin/sh");
                cmd.args(["-c", r#"echo '{"error":"offline"}'"#]);
                cmd
            }),
        );
        search.configure(Some("minilm-l6"), true);
        wait_for(&search, |s| matches!(s, ModelStatus::Failed { .. }));
        assert_eq!(
            search.status(),
            ModelStatus::Failed { model: "minilm-l6".into(), error: "offline".into() }
        );
    }

    #[test]
    fn every_model_is_one_whose_embeddings_survive_batching() {
        for m in MODELS {
            assert_eq!(
                TextEmbedding::get_quantization_mode(&m.embedding),
                fastembed::QuantizationMode::None,
                "{}",
                m.id
            );
            assert!(m.floor < m.ceiling);
        }
        assert!(model(DEFAULT_MODEL).is_some());
    }
}
