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

use crate::tasks::{TaskContext, TaskKind, TaskStatus, Tasks};

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
    /// The same for one word of the query against one word on a screen,
    /// measured with the example's `words` mode. Single words sit higher
    /// than passages: a synonym at 0.55 to 0.9 ("developer" for "engineer"
    /// 0.56, "login" for "signin" 0.79), an unrelated word at up to 0.5,
    /// so the floor sits just above that.
    word_floor: f32,
    word_ceiling: f32,
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
        word_floor: 0.52,
        word_ceiling: 0.80,
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
        word_floor: 0.50,
        word_ceiling: 0.80,
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
        word_floor: 0.52,
        word_ceiling: 0.85,
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
    /// reading it from disk. While downloading, how many bytes are in, of
    /// roughly how many.
    Loading { model: String, downloaded: Option<u64>, size: Option<u64> },
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
    /// The text that matched: the line with the query's words or with a
    /// word close to one of them, or the chunk closest in meaning.
    pub snippet: String,
    pub matched_by: MatchedBy,
    /// What to highlight in `snippet`, as `[start, end)` in UTF-16 code
    /// units, as JavaScript indexes a string. Empty for a match by meaning
    /// until `ScreenSearch::marks` has found which words in it are closest
    /// to the query: a word it shares with the query is often not what
    /// matched, and highlighting it read as a literal match.
    pub marks: Vec<[u32; 2]>,
}

/// Which of the three scores found a screen (`ScreenSearch::search`).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MatchedBy {
    /// The query's own words are on it.
    Text,
    /// A word close to one of the query's is on it.
    Word,
    /// A few lines on it are close to the query as a whole.
    Meaning,
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

/// How long a worker may take to load a model it has on disk. The large
/// one takes two seconds.
const LOAD_TIMEOUT: Duration = Duration::from_secs(60);

/// How long a download may go without a byte before it is given up. Not a
/// limit on the whole download, which on a slow connection takes minutes.
const STALL_TIMEOUT: Duration = Duration::from_secs(60);

/// How long one batch of chunks may take. A full screen on the large model
/// takes a few seconds.
const EMBED_TIMEOUT: Duration = Duration::from_secs(60);

/// Queries shorter than this are left to the branch names.
pub const MIN_QUERY: usize = 3;

/// A query of at most this many words (stopwords aside) is also matched
/// word by word.
const SHORT_QUERY: usize = 2;

/// Words taken from one screen for matching word by word, newest first. A
/// full screen of prose has two or three hundred.
const SCREEN_WORDS: usize = 400;

/// How much higher a single line has to score than a chunk to count as
/// much. Short texts spread wider: unrelated lines reach 0.27 where chunks
/// stay under 0.2.
const LINE_MARGIN: f32 = 0.07;

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

/// Where models are kept, and the task registry a download is shown in.
struct Downloads {
    cache_dir: PathBuf,
    tasks: Arc<Tasks>,
}

pub struct ScreenSearch {
    inner: Mutex<Inner>,
    /// `None` in tests, which have no cache and no tasks.
    downloads: Option<Downloads>,
    /// Vectors by text (chunks, words, word runs, queries), for the model
    /// in `inner`. It outlives the worker, so one started again after
    /// being idle embeds only what changed meanwhile.
    embeddings: Mutex<Cache>,
    on_status: OnStatus,
    spawn: Spawn,
}

impl ScreenSearch {
    /// Workers are this binary, started with `WORKER_FLAG`, keeping models
    /// in `cache_dir`. A model that has to be downloaded first is a task in
    /// `tasks`.
    pub fn new(cache_dir: PathBuf, tasks: Arc<Tasks>, on_status: OnStatus) -> Arc<Self> {
        let dir = cache_dir.clone();
        Self::with_spawn(
            Some(Downloads { cache_dir, tasks }),
            on_status,
            Box::new(move |model: &Model| {
                let exe = std::env::current_exe().unwrap_or_else(|_| PathBuf::from("parallelist"));
                let mut cmd = Command::new(exe);
                cmd.arg(WORKER_FLAG).arg(model.id).arg(&dir);
                cmd
            }),
        )
    }

    fn with_spawn(downloads: Option<Downloads>, on_status: OnStatus, spawn: Spawn) -> Arc<Self> {
        let search = Arc::new(Self {
            inner: Mutex::new(Inner {
                wanted: None,
                status: ModelStatus::Off,
                worker: None,
                failed_at: None,
                last_used: Instant::now(),
                generation: 0,
            }),
            downloads,
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
            *self.embeddings.lock().unwrap() = Cache::default();
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
        let download = self
            .downloads
            .as_ref()
            .filter(|d| !d.has(model))
            .map(|d| Download::start(d, model));
        let size = download.as_ref().map(|d| d.size);
        self.set_status(
            inner,
            ModelStatus::Loading { model: model.id.into(), downloaded: size.map(|_| 0), size },
        );
        let this = Arc::clone(self);
        std::thread::spawn(move || {
            let begun = Instant::now();
            let mut progress = (0, Instant::now());
            let started = Worker::start((this.spawn)(model), &mut || {
                if this.inner.lock().unwrap().generation != generation {
                    return Some(StartError::Cancelled);
                }
                let Some(download) = &download else {
                    return (begun.elapsed() > LOAD_TIMEOUT)
                        .then(|| StartError::Failed(format!("{} took too long to load", model.name)));
                };
                let bytes = download.bytes();
                if bytes > progress.0 {
                    progress = (bytes, Instant::now());
                    download.report(bytes);
                    this.downloading(generation, model, bytes, download.size);
                } else if progress.1.elapsed() > STALL_TIMEOUT {
                    return Some(StartError::Failed("the download stalled".into()));
                }
                None
            });
            if let Some(download) = download {
                download.finish(&started);
            }
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
                Err(StartError::Cancelled) => return,
                Err(StartError::Failed(error)) => {
                    inner.failed_at = Some(Instant::now());
                    ModelStatus::Failed { model: model.id.into(), error }
                }
            };
            this.set_status(&mut inner, status);
        });
    }

    /// How far a download has got, for the palette and Settings.
    fn downloading(&self, generation: u64, model: &Model, bytes: u64, size: u64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation == generation {
            let status = ModelStatus::Loading {
                model: model.id.into(),
                downloaded: Some(bytes.min(size)),
                size: Some(size),
            };
            self.set_status(&mut inner, status);
        }
    }

    /// A worker that stopped answering mid-search is gone. Its model loaded
    /// once, so the next search starts another straight away; one that
    /// then fails to load waits `RETRY_AFTER` like any other.
    fn worker_died(&self, generation: u64) {
        let mut inner = self.inner.lock().unwrap();
        if inner.generation != generation {
            return;
        }
        inner.worker = None;
        if let Some(model) = inner.wanted {
            self.set_status(&mut inner, ModelStatus::Idle { model: model.id.into() });
        }
    }

    /// Score every screen against `query`, once the worker is ready, which
    /// this starts if it isn't. Until then nothing is found: the palette
    /// says where the model is instead (`Found::status`). Once it is ready
    /// a screen counts by the best of three scores, each 0..1:
    ///
    /// - **Text**: the share of the query's words found on it.
    /// - **Word**, for a query of one or two words: how close each of them
    ///   is to the closest word on it. Embedded as a passage, "why did the
    ///   developer trust the clean badge?" is about trusting badges, and
    ///   scores no higher against "engineer" than unrelated text does; the
    ///   word "developer" on its own scores well above it.
    /// - **Meaning**: how close its closest chunk is to the query as a
    ///   whole, which is what finds a question phrased differently.
    pub fn search(self: &Arc<Self>, query: &str, screens: &[Screen]) -> Found {
        let (status, worker, model, generation) = {
            let mut inner = self.inner.lock().unwrap();
            inner.last_used = Instant::now();
            self.start_worker(&mut inner);
            (inner.status.clone(), inner.worker.clone(), inner.wanted, inner.generation)
        };
        let query = query.trim();
        let ready = worker.zip(model).filter(|_| matches!(status, ModelStatus::Ready { .. }));
        let mut hits = Vec::new();
        if let Some((worker, model)) = ready.filter(|_| query.chars().count() >= MIN_QUERY) {
            match self.score(&worker, model, query, screens) {
                Ok(found) => hits = found,
                Err(_) => self.worker_died(generation),
            }
        }
        Found { query: query.to_string(), status: self.status(), hits }
    }

    fn score(
        &self,
        worker: &Mutex<Worker>,
        model: &Model,
        query: &str,
        screens: &[Screen],
    ) -> Result<Vec<Hit>, String> {
        let words = query_words(query);
        let by_word = (1..=SHORT_QUERY).contains(&words.len());
        let prepared: Vec<Prepared> = screens.iter().map(|s| prepare(s, by_word)).collect();

        // Meaning isn't asked of a single word, which against a passage
        // scores as much at random ("engineer" found "Welcome to the
        // agent") as where it fits.
        let by_meaning = words.len() != 1;

        // Everything to embed, in one request: the query, every chunk and
        // line, and for a short query its words and every word on every
        // screen.
        let mut texts: Vec<&str> = vec![query];
        if by_meaning {
            texts.extend(prepared.iter().flat_map(|s| s.chunks.iter().map(String::as_str)));
            texts.extend(prepared.iter().flat_map(|s| s.sentences().map(String::as_str)));
        }
        if by_word {
            texts.extend(words.iter().map(String::as_str));
            texts.extend(prepared.iter().flat_map(|s| s.words.iter().map(String::as_str)));
        }
        self.embeddings.lock().unwrap().round += 1;
        let vectors = self.vectors(worker, &texts)?;
        let vector = |text: &str| vectors.get(&hash(text));
        let query_vector = vector(query).ok_or("no query vector")?;
        let scaled = |sim: f32, floor: f32, ceiling: f32| ((sim - floor) / (ceiling - floor)).clamp(0.0, 1.0);

        let mut hits = Vec::new();
        for screen in &prepared {
            // Text.
            let (share, line) = literal(&words, &screen.lines);
            let mut best = (share, MatchedBy::Text, line.map(|l| screen.lines[l].clone()));

            // Word: each of the query's words against its closest word here,
            // averaged, so both of two words have to be found.
            if by_word && !screen.words.is_empty() {
                let mut total = 0.0;
                let mut closest: Option<(f32, &str)> = None;
                for word in &words {
                    let Some(w) = vector(word) else { continue };
                    let near = screen
                        .words
                        .iter()
                        .filter_map(|other| Some((dot(w, vector(other)?), other.as_str())))
                        .max_by(|a, b| a.0.total_cmp(&b.0));
                    if let Some((sim, other)) = near {
                        total += scaled(sim, model.word_floor, model.word_ceiling);
                        if closest.is_none_or(|(s, _)| sim > s) {
                            closest = Some((sim, other));
                        }
                    }
                }
                let score = total / words.len() as f32;
                if score > best.0 {
                    let line = closest.and_then(|(_, w)| newest_line_with(&screen.lines, w));
                    best = (score, MatchedBy::Word, line.map(|l| screen.lines[l].clone()));
                }
            }

            // Meaning: the chunk or the line closest to the query as a whole.
            // A line said in a chunk of other things is diluted there ("If
            // you want something real to work on next", under a list of
            // jokes); on its own it scores higher, and so does noise, hence
            // its higher floor.
            if by_meaning {
                let mut closest: Option<(f32, &str)> = None;
                let chunks = screen.chunks.iter().map(|t| (t, 0.0));
                let lines = screen.sentences().map(|t| (t, LINE_MARGIN));
                for (text, margin) in chunks.chain(lines) {
                    let Some(v) = vector(text) else { continue };
                    let score = scaled(dot(query_vector, v), model.floor + margin, model.ceiling + margin);
                    if closest.is_none_or(|(s, _)| score > s) {
                        closest = Some((score, text));
                    }
                }
                if let Some((score, text)) = closest.filter(|(score, _)| *score > best.0) {
                    best = (score, MatchedBy::Meaning, Some(text.replace('\n', " · ")));
                }
            }

            let (relevance, matched_by, snippet) = best;
            if relevance < MIN_RELEVANCE {
                continue;
            }
            let snippet = clip(&snippet.unwrap_or_default(), SNIPPET_CHARS);
            let marks = match matched_by {
                MatchedBy::Text => words.iter().flat_map(|w| occurrences(&snippet, w, Bound::Start)).collect(),
                MatchedBy::Word => screen
                    .words
                    .iter()
                    .filter(|other| {
                        words.iter().any(|w| match (vector(w), vector(other)) {
                            (Some(a), Some(b)) => dot(a, b) >= model.word_floor,
                            _ => false,
                        })
                    })
                    .flat_map(|other| occurrences(&snippet, other, Bound::Whole))
                    .collect(),
                MatchedBy::Meaning => Vec::new(),
            };
            hits.push(Hit {
                path: screen.path.clone(),
                relevance,
                snippet: snippet.clone(),
                matched_by,
                marks: marks.into_iter().map(|(a, b)| utf16_range(&snippet, a, b)).collect(),
            });
        }
        hits.sort_by(|a, b| b.relevance.total_cmp(&a.relevance));
        Ok(hits)
    }

    /// For each snippet a match by meaning found, the run of one to three
    /// of its words closest to `query`, as ranges to highlight (see
    /// `Hit::marks`); empty when none is close enough to point at. Asked by
    /// the palette after the results are on screen, since a snippet's runs
    /// take a fraction of a second to embed the first time.
    pub fn marks(&self, query: &str, snippets: &[String]) -> Vec<Vec<[u32; 2]>> {
        let (worker, model) = {
            let inner = self.inner.lock().unwrap();
            if !matches!(inner.status, ModelStatus::Ready { .. }) {
                return vec![Vec::new(); snippets.len()];
            }
            match (inner.worker.clone(), inner.wanted) {
                (Some(worker), Some(model)) => (worker, model),
                _ => return vec![Vec::new(); snippets.len()],
            }
        };
        let query = query.trim();
        let spans: Vec<Vec<(usize, usize)>> = snippets.iter().map(|s| spans(s)).collect();
        let mut texts: Vec<&str> = vec![query];
        for (snippet, runs) in snippets.iter().zip(&spans) {
            texts.extend(runs.iter().map(|&(a, b)| &snippet[a..b]));
        }
        let Ok(vectors) = self.vectors(&worker, &texts) else {
            return vec![Vec::new(); snippets.len()];
        };
        let Some(q) = vectors.get(&hash(query)) else {
            return vec![Vec::new(); snippets.len()];
        };
        snippets
            .iter()
            .zip(&spans)
            .map(|(snippet, runs)| {
                // The closest run; among equals the shortest, which says
                // most precisely what matched.
                let best = runs
                    .iter()
                    .filter_map(|&(a, b)| Some((dot(q, vectors.get(&hash(&snippet[a..b]))?), a, b)))
                    .max_by(|x, y| x.0.total_cmp(&y.0).then((y.2 - y.1).cmp(&(x.2 - x.1))));
                match best {
                    Some((sim, a, b)) if sim >= model.floor => vec![utf16_range(snippet, a, b)],
                    _ => Vec::new(),
                }
            })
            .collect()
    }

    /// Vectors for `texts`, normalised, by the hash of their text: from the
    /// cache where it has them, the rest from the worker in one request.
    fn vectors(
        &self,
        worker: &Mutex<Worker>,
        texts: &[&str],
    ) -> Result<HashMap<u64, Arc<[f32]>>, String> {
        let mut found = HashMap::new();
        let mut missing: Vec<&str> = Vec::new();
        {
            let mut cache = self.embeddings.lock().unwrap();
            let round = cache.round;
            for text in texts {
                let key = hash(text);
                if found.contains_key(&key) {
                    continue;
                }
                if let Some((vector, used)) = cache.vectors.get_mut(&key) {
                    *used = round;
                    found.insert(key, Arc::clone(vector));
                } else if !missing.contains(text) {
                    missing.push(text);
                }
            }
        }
        if !missing.is_empty() {
            let fresh = worker.lock().unwrap().embed(&missing)?;
            let mut cache = self.embeddings.lock().unwrap();
            let round = cache.round;
            for (text, vector) in missing.iter().zip(fresh) {
                let vector = normalized(vector);
                cache.vectors.insert(hash(text), (Arc::clone(&vector), round));
                found.insert(hash(text), vector);
            }
            cache.prune();
        }
        Ok(found)
    }
}

/// Vectors by the hash of their text, each with the search that last used
/// it, so what no recent search has used can go.
#[derive(Default)]
struct Cache {
    vectors: HashMap<u64, (Arc<[f32]>, u64)>,
    round: u64,
}

/// How many vectors the cache holds before it drops what the last few
/// searches didn't use. A screen's chunks and words are a few hundred.
const CACHE_LIMIT: usize = 50_000;
const CACHE_ROUNDS: u64 = 20;

impl Cache {
    fn prune(&mut self) {
        if self.vectors.len() > CACHE_LIMIT {
            let keep_from = self.round.saturating_sub(CACHE_ROUNDS);
            self.vectors.retain(|_, (_, used)| *used >= keep_from);
        }
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

/// Why a worker didn't start.
#[derive(Debug)]
enum StartError {
    /// The setting changed while it started: off, or another model.
    Cancelled,
    Failed(String),
}

/// A model being downloaded, shown as a task.
struct Download {
    tasks: Arc<Tasks>,
    id: String,
    name: &'static str,
    /// Where hf-hub writes it, `.part` files included, so its size is the
    /// progress.
    blobs: PathBuf,
    /// Roughly what it comes to, in bytes.
    size: u64,
}

const MB: u64 = 1_000_000;

impl Download {
    fn start(downloads: &Downloads, model: &'static Model) -> Self {
        let id = downloads.tasks.add(
            "",
            TaskKind::Download,
            "Downloading the screen search model".into(),
            TaskContext::default(),
            TaskStatus::Running,
        );
        let download = Download {
            tasks: Arc::clone(&downloads.tasks),
            id,
            name: model.name,
            blobs: repo_dir(&downloads.cache_dir, model).join("blobs"),
            size: u64::from(model.download_mb) * MB,
        };
        download.report(download.bytes());
        download
    }

    fn bytes(&self) -> u64 {
        std::fs::read_dir(&self.blobs)
            .map(|entries| {
                entries
                    .flatten()
                    .filter_map(|e| e.metadata().ok())
                    .filter(|m| m.is_file())
                    .map(|m| m.len())
                    .sum()
            })
            .unwrap_or(0)
    }

    fn report(&self, bytes: u64) {
        let line = format!("{} · {} of {} MB", self.name, bytes.min(self.size) / MB, self.size / MB);
        self.tasks.keyed_line(&self.id, "progress", &line, false);
    }

    fn finish(self, started: &Result<Worker, StartError>) {
        let result = match started {
            Ok(_) => {
                let line = format!("Downloaded {}, {} MB", self.name, self.bytes() / MB);
                self.tasks.keyed_line(&self.id, "progress", &line, false);
                Ok(())
            }
            Err(StartError::Cancelled) => {
                // A part file stays behind, and the next download resumes it.
                let line = "Stopped, as the setting changed";
                self.tasks.keyed_line(&self.id, "progress", line, false);
                Ok(())
            }
            Err(StartError::Failed(error)) => Err(error.clone()),
        };
        self.tasks.finish(&self.id, result);
    }
}

impl Downloads {
    /// Whether the model's files are all on disk, so starting it downloads
    /// nothing.
    fn has(&self, model: &Model) -> bool {
        let Ok(info) = TextEmbedding::get_model_info(&model.embedding) else {
            return false;
        };
        let Ok(snapshots) = std::fs::read_dir(repo_dir(&self.cache_dir, model).join("snapshots")) else {
            return false;
        };
        snapshots.flatten().any(|snapshot| {
            std::iter::once(&info.model_file)
                .chain(&info.additional_files)
                .all(|file| snapshot.path().join(file).exists())
        })
    }
}

/// Where hf-hub keeps a model's repository: `models--<org>--<name>`.
fn repo_dir(cache_dir: &std::path::Path, model: &Model) -> PathBuf {
    let code = TextEmbedding::get_model_info(&model.embedding)
        .map(|info| info.model_code.replace('/', "--"))
        .unwrap_or_default();
    cache_dir.join(format!("models--{code}"))
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
    /// Start `cmd` and wait until it says its model is loaded. `check` is
    /// asked twice a second meanwhile, and stops the wait, and the worker,
    /// when it says why.
    fn start(
        mut cmd: Command,
        check: &mut dyn FnMut() -> Option<StartError>,
    ) -> Result<Self, StartError> {
        cmd.stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .process_group(0);
        let mut child = cmd
            .spawn()
            .map_err(|e| StartError::Failed(format!("could not start the model: {e}")))?;
        let stdin = child.stdin.take().ok_or(StartError::Failed("no stdin".into()))?;
        let stdout = child.stdout.take().ok_or(StartError::Failed("no stdout".into()))?;
        let (tx, lines) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines().map_while(Result::ok) {
                if tx.send(line).is_err() {
                    return;
                }
            }
        });
        let mut worker = Worker { child, stdin, lines };
        loop {
            match worker.reply(Duration::from_millis(500)) {
                Ok(Reply::Ready) => return Ok(worker),
                Ok(Reply::Error(e)) => return Err(StartError::Failed(e)),
                Ok(Reply::Vectors(_)) => {
                    return Err(StartError::Failed("the model answered before it was asked".into()))
                }
                Err(Silence::Waiting) => {}
                Err(Silence::Gone(e)) => return Err(StartError::Failed(e)),
            }
            // Dropping the worker kills it.
            if let Some(stop) = check() {
                return Err(stop);
            }
        }
    }

    fn reply(&mut self, timeout: Duration) -> Result<Reply, Silence> {
        let line = self.lines.recv_timeout(timeout).map_err(|e| match e {
            RecvTimeoutError::Timeout => Silence::Waiting,
            RecvTimeoutError::Disconnected => Silence::Gone("the model stopped".into()),
        })?;
        serde_json::from_str(&line)
            .map_err(|e| Silence::Gone(format!("the model answered nonsense: {e}")))
    }

    /// One vector per text, in order.
    fn embed(&mut self, texts: &[&str]) -> Result<Vec<Vec<f32>>, String> {
        let mut request = serde_json::to_string(texts).map_err(|e| e.to_string())?;
        request.push('\n');
        self.stdin
            .write_all(request.as_bytes())
            .and_then(|()| self.stdin.flush())
            .map_err(|_| "the model stopped".to_string())?;
        let reply = self.reply(EMBED_TIMEOUT).map_err(|silence| match silence {
            Silence::Waiting => "the model stopped responding".to_string(),
            Silence::Gone(e) => e,
        })?;
        match reply {
            Reply::Vectors(vectors) if vectors.len() == texts.len() => Ok(vectors),
            Reply::Vectors(_) => Err("the model answered for the wrong number of texts".into()),
            Reply::Error(e) => Err(e),
            Reply::Ready => Err("the model answered out of turn".into()),
        }
    }
}

/// No reply: none yet, or none coming.
enum Silence {
    Waiting,
    Gone(String),
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
            Ok(texts) => match embedder.embed(&texts, Some(batch_for(&texts))) {
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

/// Single words run many to a batch: they pad to a few tokens, where a
/// chunk of lines pads to a hundred or more.
fn batch_for(texts: &[String]) -> usize {
    if texts.iter().all(|t| t.len() <= 40) {
        64
    } else {
        EMBED_BATCH
    }
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
    /// Its distinct words, for matching word by word; empty unless asked.
    words: Vec<String>,
}

impl Prepared {
    /// The lines that say enough to be matched by meaning on their own:
    /// three words or more. A line like `failures:` is close to anything
    /// about failing, and says less than the line under it.
    fn sentences(&self) -> impl Iterator<Item = &String> {
        self.lines.iter().filter(|l| l.split_whitespace().count() >= 3)
    }
}

fn prepare(screen: &Screen, with_words: bool) -> Prepared {
    let lines = clean_lines(&screen.text);
    let chunks = chunks(&lines);
    let words = if with_words { vocabulary(&lines) } else { Vec::new() };
    Prepared { path: screen.path.clone(), lines, chunks, words }
}

/// A screen's distinct words worth comparing with a query's: three letters
/// or more, letters only (so no hashes, numbers or paths), no stopwords,
/// lowercased, newest first, at most `SCREEN_WORDS`.
fn vocabulary(lines: &[String]) -> Vec<String> {
    let mut seen = std::collections::HashSet::new();
    let mut words = Vec::new();
    for line in lines.iter().rev() {
        for word in line.split(|c: char| !c.is_alphabetic()) {
            if word.chars().count() < 3 {
                continue;
            }
            let word = word.to_lowercase();
            if STOPWORDS.contains(&word.as_str()) || !seen.insert(word.clone()) {
                continue;
            }
            words.push(word);
            if words.len() == SCREEN_WORDS {
                return words;
            }
        }
    }
    words
}

/// The newest line holding `word` as a word of its own.
fn newest_line_with(lines: &[String], word: &str) -> Option<usize> {
    lines.iter().rposition(|line| !occurrences(line, word, Bound::Whole).is_empty())
}

/// Where a word of the query, or of a screen, may sit in a text.
#[derive(Clone, Copy, PartialEq)]
enum Bound {
    /// At the start of a word: "refund" in "refunds", not "work" in
    /// "Network".
    Start,
    /// As a word of its own.
    Whole,
}

/// Where `needle` occurs in `text`, ignoring ASCII case, as byte ranges.
fn occurrences(text: &str, needle: &str, bound: Bound) -> Vec<(usize, usize)> {
    let hay = text.to_ascii_lowercase();
    let needle = needle.to_ascii_lowercase();
    if needle.is_empty() {
        return Vec::new();
    }
    let letter = |c: Option<char>| c.is_some_and(char::is_alphanumeric);
    let mut found = Vec::new();
    let mut from = 0;
    while let Some(i) = hay[from..].find(&needle) {
        let (a, b) = (from + i, from + i + needle.len());
        let starts = !letter(hay[..a].chars().next_back());
        if starts && (bound == Bound::Start || !letter(hay[b..].chars().next())) {
            found.push((a, b));
        }
        from = b;
    }
    found
}

/// The runs of one to three words in a snippet, as byte ranges, with
/// punctuation trimmed from their ends; runs across the ` · ` between
/// lines, or of stopwords alone, are left out.
fn spans(snippet: &str) -> Vec<(usize, usize)> {
    let mut words: Vec<(usize, usize)> = Vec::new();
    let mut start = None;
    for (i, c) in snippet.char_indices().chain(std::iter::once((snippet.len(), ' '))) {
        match (c.is_whitespace(), start) {
            (false, None) => start = Some(i),
            (true, Some(a)) => {
                words.push((a, i));
                start = None;
            }
            _ => {}
        }
    }
    let trim = |(a, b): (usize, usize)| {
        let text = &snippet[a..b];
        let lead = text.len() - text.trim_start_matches(|c: char| !c.is_alphanumeric()).len();
        let tail = text.len() - text.trim_end_matches(|c: char| !c.is_alphanumeric()).len();
        (a + lead, b.saturating_sub(tail).max(a + lead))
    };
    let mut runs = Vec::new();
    for n in 1..=3 {
        for group in words.windows(n) {
            let texts: Vec<&str> = group.iter().map(|&(a, b)| &snippet[a..b]).collect();
            if texts.contains(&"·") {
                continue;
            }
            let content = texts.iter().any(|t| {
                let t = t.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
                t.chars().count() >= 3 && !STOPWORDS.contains(&t.as_str())
            });
            let (a, b) = trim((group[0].0, group[n - 1].1));
            if content && a < b {
                runs.push((a, b));
            }
        }
    }
    runs
}

/// A byte range of `text` as UTF-16 code units, which is how JavaScript
/// indexes the string.
fn utf16_range(text: &str, a: usize, b: usize) -> [u32; 2] {
    let start = text[..a].encode_utf16().count();
    [start as u32, (start + text[a..b].encode_utf16().count()) as u32]
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

/// How much of the query is on the screen: the share of `words` found at
/// the start of a word, squared, so one word in three counts for little
/// (0.11) and all of them for everything. With the line holding most of
/// them, the lowest on the screen among equals (the newest).
fn literal(words: &[String], lines: &[String]) -> (f32, Option<usize>) {
    if words.is_empty() {
        return (0.0, None);
    }
    let on = |line: &str, word: &str| !occurrences(line, word, Bound::Start).is_empty();
    let found = words.iter().filter(|w| lines.iter().any(|l| on(l, w))).count();
    let best = lines
        .iter()
        .enumerate()
        .map(|(i, l)| (words.iter().filter(|w| on(l, w)).count(), i))
        .filter(|(n, _)| *n > 0)
        .max()
        .map(|(_, i)| i);
    let share = found as f32 / words.len() as f32;
    (share * share, best)
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
        screens.iter().map(|s| embed(&prepare(s, false).chunks)).collect::<Result<_, _>>()?;
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

/// For calibrating a model's `word_floor` and `word_ceiling` (the
/// example's `words` mode): for each query word, the closest word on each
/// screen, and how close.
pub fn word_similarities(
    model_id: &str,
    cache_dir: PathBuf,
    queries: &[&str],
    screens: &[Screen],
) -> Result<Vec<Vec<(f32, String)>>, String> {
    let model = model(model_id).ok_or("unknown model")?;
    let mut embedder = TextEmbedding::try_new(model.options(cache_dir)).map_err(|e| e.to_string())?;
    let mut embed = |texts: &[String]| -> Result<Vec<Arc<[f32]>>, String> {
        Ok(embedder
            .embed(texts, Some(64))
            .map_err(|e| e.to_string())?
            .into_iter()
            .map(normalized)
            .collect())
    };
    let vocabularies: Vec<Vec<String>> = screens.iter().map(|s| prepare(s, true).words).collect();
    let vectors: Vec<Vec<Arc<[f32]>>> = vocabularies.iter().map(|v| embed(v)).collect::<Result<_, _>>()?;
    let query_vectors = embed(&queries.iter().map(|q| q.to_string()).collect::<Vec<_>>())?;
    Ok(query_vectors
        .iter()
        .map(|q| {
            vocabularies
                .iter()
                .zip(&vectors)
                .map(|(words, vs)| {
                    words
                        .iter()
                        .zip(vs)
                        .map(|(w, v)| (dot(q, v), w.clone()))
                        .max_by(|a, b| a.0.total_cmp(&b.0))
                        .unwrap_or((0.0, String::new()))
                })
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
        let (score, line) = literal(&words, &lines);
        assert!((score - 4.0 / 9.0).abs() < 1e-6, "two words of three, squared");
        assert_eq!(line, Some(2));
        assert_eq!(literal(&query_words("on it"), &lines), (0.0, None), "too short to say anything");
        let net = ["Network: use --host".to_string(), "a working tree".to_string()];
        assert_eq!(literal(&query_words("work"), &net), (1.0, Some(1)), "at a word's start only");
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

    /// A worker that answers like the real one, from a shell, with a
    /// made-up model that knows three things: anything mentioning "vite" is
    /// [1, 0, 0], "engineer" and "developer" mean the same, [0, 1, 0], and
    /// anything failing is [0, 0, 1]. Everything else is nothing, [0, 0, 0].
    fn fake_worker() -> Command {
        let mut cmd = Command::new("/bin/sh");
        cmd.arg("-c").arg(
            r#"echo '"ready"'
            while read -r line; do
              out=""
              rest="${line#[\"}"
              rest="${rest%\"]}"
              while :; do
                item="${rest%%\",\"*}"
                case "$item" in
                  *[Vv][Ii][Tt][Ee]*) v="[1,0,0]";;
                  *[Ee]ngineer*|*[Dd]eveloper*) v="[0,1,0]";;
                  *[Ff][Aa][Ii][Ll]*) v="[0,0,1]";;
                  *) v="[0,0,0]";;
                esac
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
        let search = ScreenSearch::with_spawn(None, Box::new(|_| {}), Box::new(|_| fake_worker()));
        assert!(search.search("failed", &screens()).hits.is_empty());
        search.configure(Some("minilm-l6"), false);
        assert!(search.search("te", &screens()).hits.is_empty());
    }

    #[test]
    fn until_the_worker_is_ready_nothing_is_found() {
        let search = ScreenSearch::with_spawn(
            None,
            Box::new(|_| {}),
            Box::new(|_| {
                let mut cmd = Command::new("/bin/sh");
                cmd.args(["-c", "sleep 5"]);
                cmd
            }),
        );
        search.configure(Some("minilm-l6"), false);
        let found = search.search("test failed", &screens());
        assert_eq!(
            found.status,
            ModelStatus::Loading { model: "minilm-l6".into(), downloaded: None, size: None }
        );
        assert!(found.hits.is_empty(), "not even the literal match: {:?}", found.hits);
        search.configure(None, false);
    }

    #[test]
    fn a_ready_worker_matches_by_meaning() {
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&statuses);
        let search = ScreenSearch::with_spawn(
            None,
            Box::new(move |s| seen.lock().unwrap().push(s.clone())),
            Box::new(|_| fake_worker()),
        );
        search.configure(Some("minilm-l6"), true);
        wait_for(&search, |s| matches!(s, ModelStatus::Ready { .. }));
        // Half the words are on /b's screen; its meaning matches fully.
        let found = search.search("Vite thing", &screens());
        let b = found.hits.iter().find(|h| h.path == "/b").expect("/b by meaning");
        assert_eq!(b.relevance, 1.0);
        assert_eq!(b.matched_by, MatchedBy::Meaning);
        assert!(b.marks.is_empty(), "found afterwards, by `marks`");
        assert!(b.snippet.contains("VITE ready"), "{}", b.snippet);
        assert!(found.hits.iter().all(|h| h.path != "/a"), "{:?}", found.hits);
        assert_eq!(
            *statuses.lock().unwrap(),
            [
                ModelStatus::Idle { model: "minilm-l6".into() },
                ModelStatus::Loading { model: "minilm-l6".into(), downloaded: None, size: None },
                ModelStatus::Ready { model: "minilm-l6".into() },
            ]
        );
        // Turning it off stops the worker.
        search.configure(None, false);
        assert_eq!(search.status(), ModelStatus::Off);
    }

    #[test]
    fn a_worker_that_dies_is_started_again_on_the_next_search() {
        let starts = Arc::new(Mutex::new(0));
        let counted = Arc::clone(&starts);
        let search = ScreenSearch::with_spawn(
            None,
            Box::new(|_| {}),
            Box::new(move |_| {
                *counted.lock().unwrap() += 1;
                let mut cmd = Command::new("/bin/sh");
                cmd.args(["-c", "echo '\"ready\"'; read -r line; exit 3"]);
                cmd
            }),
        );
        search.configure(Some("minilm-l6"), true);
        wait_for(&search, |s| matches!(s, ModelStatus::Ready { .. }));
        let found = search.search("hmr update", &screens());
        assert!(found.hits.is_empty(), "{:?}", found.hits);
        assert_eq!(found.status, ModelStatus::Idle { model: "minilm-l6".into() });
        // Loaded once, so not held back as a failure would be.
        search.search("hmr updates", &screens());
        wait_for(&search, |s| matches!(s, ModelStatus::Ready { .. }));
        assert_eq!(*starts.lock().unwrap(), 2);
    }

    /// A stand-in for a worker downloading its model: it writes `chunks`
    /// MB into the model's blobs folder, a chunk at a time, then says it is
    /// ready.
    fn downloading_worker(blobs: PathBuf, chunks: u32) -> Spawn {
        Box::new(move |_| {
            let mut cmd = Command::new("/bin/sh");
            cmd.arg("-c").arg(format!(
                "mkdir -p '{dir}'; i=0; while [ $i -lt {chunks} ]; do \
                 head -c 1000000 /dev/zero >> '{dir}/blob.sync.part'; sleep 0.6; i=$((i+1)); done; \
                 echo '\"ready\"'; cat > /dev/null",
                dir = blobs.display()
            ));
            cmd
        })
    }

    fn cache(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("parallelist-models-{name}-{}", std::process::id()));
        std::fs::remove_dir_all(&dir).ok();
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn wait_for_task(tasks: &Tasks, done: impl Fn(&crate::tasks::Task) -> bool) -> crate::tasks::Task {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(task) = tasks.list().into_iter().find(|t| done(t)) {
                return task;
            }
            assert!(Instant::now() < deadline, "no such task in {:?}", tasks.list().len());
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    #[test]
    fn a_model_not_on_disk_is_downloaded_as_a_task() {
        let cache_dir = cache("download");
        let model = &MODELS[0];
        let blobs = repo_dir(&cache_dir, model).join("blobs");
        let tasks = Arc::new(Tasks::new(|_| {}));
        let statuses = Arc::new(Mutex::new(Vec::new()));
        let seen = Arc::clone(&statuses);
        let downloads = Downloads { cache_dir: cache_dir.clone(), tasks: Arc::clone(&tasks) };
        assert!(!downloads.has(model));
        let search = ScreenSearch::with_spawn(
            Some(downloads),
            Box::new(move |s| seen.lock().unwrap().push(s.clone())),
            downloading_worker(blobs, 3),
        );
        search.configure(Some(model.id), true);
        wait_for_task(&tasks, |t| t.header.status == TaskStatus::Done);
        wait_for(&search, |s| matches!(s, ModelStatus::Ready { .. }));

        let task = tasks.list().pop().unwrap();
        assert_eq!(task.header.kind, TaskKind::Download);
        assert_eq!(task.header.project_id, "");
        assert_eq!(task.log.back().unwrap().text, "Downloaded all-MiniLM-L6-v2, 3 MB");
        let progress: Vec<u64> = statuses
            .lock()
            .unwrap()
            .iter()
            .filter_map(|s| match s {
                ModelStatus::Loading { downloaded, size, .. } => {
                    assert_eq!(*size, Some(90 * MB));
                    *downloaded
                }
                _ => None,
            })
            .collect();
        assert!(progress.windows(2).all(|w| w[0] < w[1]), "counts up: {progress:?}");
        assert!(progress.last() >= Some(&(2 * MB)), "{progress:?}");

        // Once its files are there, nothing is downloaded and no task shown.
        let info = TextEmbedding::get_model_info(&model.embedding).unwrap();
        let snapshot = repo_dir(&cache_dir, model).join("snapshots/abc");
        for file in std::iter::once(&info.model_file).chain(&info.additional_files) {
            std::fs::create_dir_all(snapshot.join(file).parent().unwrap()).unwrap();
            std::fs::write(snapshot.join(file), "x").unwrap();
        }
        let tasks = Arc::new(Tasks::new(|_| {}));
        let downloads = Downloads { cache_dir: cache_dir.clone(), tasks: Arc::clone(&tasks) };
        assert!(downloads.has(model));
        let search = ScreenSearch::with_spawn(Some(downloads), Box::new(|_| {}), Box::new(|_| fake_worker()));
        search.configure(Some(model.id), true);
        wait_for(&search, |s| matches!(s, ModelStatus::Ready { .. }));
        assert!(tasks.list().is_empty());
        std::fs::remove_dir_all(&cache_dir).ok();
    }

    #[test]
    fn turning_it_off_stops_a_download() {
        let cache_dir = cache("stopped");
        let blobs = repo_dir(&cache_dir, &MODELS[0]).join("blobs");
        let tasks = Arc::new(Tasks::new(|_| {}));
        let downloads = Downloads { cache_dir: cache_dir.clone(), tasks: Arc::clone(&tasks) };
        let search = ScreenSearch::with_spawn(Some(downloads), Box::new(|_| {}), downloading_worker(blobs, 100));
        search.configure(Some(MODELS[0].id), true);
        std::thread::sleep(Duration::from_millis(800));
        search.configure(None, false);
        let task = wait_for_task(&tasks, |t| t.header.status != TaskStatus::Running);
        assert_eq!(task.header.status, TaskStatus::Done, "not a failure to point out");
        assert_eq!(task.log.back().unwrap().text, "Stopped, as the setting changed");
        assert_eq!(search.status(), ModelStatus::Off);
        std::fs::remove_dir_all(&cache_dir).ok();
    }

    fn ready(search: &Arc<ScreenSearch>) {
        search.configure(Some("minilm-l6"), true);
        wait_for(search, |s| matches!(s, ModelStatus::Ready { .. }));
    }

    #[test]
    fn a_short_query_matches_a_word_close_to_it() {
        let search = ScreenSearch::with_spawn(None, Box::new(|_| {}), Box::new(|_| fake_worker()));
        ready(&search);
        let screens = [
            Screen {
                path: "/jokes".into(),
                text: "Why did the developer trust the clean badge?\nBecause unknown defaulted to zero.\n".into(),
            },
            Screen { path: "/b".into(), text: "VITE ready in 412 ms\n".into() },
        ];
        let found = search.search("engineer", &screens);
        assert_eq!(found.hits.len(), 1, "{:?}", found.hits);
        let hit = &found.hits[0];
        assert_eq!(hit.path, "/jokes");
        assert_eq!(hit.matched_by, MatchedBy::Word);
        assert_eq!(hit.snippet, "Why did the developer trust the clean badge?");
        assert_eq!(hit.marks, [[12, 21]], "\"developer\"");
    }

    #[test]
    fn a_text_match_marks_the_query_words() {
        let search = ScreenSearch::with_spawn(None, Box::new(|_| {}), Box::new(|_| fake_worker()));
        ready(&search);
        let screens = [Screen { path: "/a".into(), text: "→ test result: ok; result kept\n".into() }];
        let hit = &search.search("result", &screens).hits[0];
        assert_eq!(hit.matched_by, MatchedBy::Text);
        // In UTF-16 units, past the arrow.
        assert_eq!(hit.marks, [[7, 13], [19, 25]]);
    }

    #[test]
    fn marks_point_at_the_closest_words_of_a_match_by_meaning() {
        let search = ScreenSearch::with_spawn(None, Box::new(|_| {}), Box::new(|_| fake_worker()));
        ready(&search);
        let snippets = ["Why did the developer trust the clean badge? · next line".to_string()];
        assert_eq!(search.marks("the engineer", &snippets), [vec![[12, 21]]]);
        assert_eq!(search.marks("nothing alike", &snippets), [Vec::<[u32; 2]>::new()]);
    }

    #[test]
    fn runs_are_words_trimmed_of_punctuation_within_a_line() {
        let snippet = "Stripe-Signature throws, the event · next";
        let runs: Vec<&str> = spans(snippet).into_iter().map(|(a, b)| &snippet[a..b]).collect();
        assert!(runs.contains(&"Stripe-Signature"));
        assert!(runs.contains(&"throws"), "comma trimmed: {runs:?}");
        assert!(runs.contains(&"throws, the event"));
        assert!(!runs.iter().any(|r| r.contains('·')), "{runs:?}");
        assert!(!runs.contains(&"the"), "stopwords alone are no run");
    }

    #[test]
    fn a_worker_that_cannot_load_says_why() {
        let search = ScreenSearch::with_spawn(
            None,
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
