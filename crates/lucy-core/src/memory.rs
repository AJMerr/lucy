//! Long-term memory: memory.md in the data folder, a Markdown list of facts about the user
//! under headings. She changes it with the `remember` and `forget` tools (edits are made line
//! by line, so whatever else you write in the file stays), and every chat's prompt carries it:
//! all of it while it's small, then the facts about you and your preferences plus the ones
//! relevant to the chat, by keyword or, with an embedding model, by meaning.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

use lucy_protocol::rpc::{MemoryFact, MemoryView};
use lucy_providers::embed::Embedder;
use regex::Regex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Up to this size the whole file goes into every prompt (about 2k tokens).
pub const PROMPT_BUDGET: usize = 8 * 1024;

/// Headings whose facts always go into the prompt.
const ALWAYS: &[&str] = &["About you", "Preferences"];

/// The headings she files facts under (others work too).
pub const HEADINGS: &[&str] = &["About you", "Preferences", "People", "Projects"];

/// Where facts go when the file has no heading above them.
const NO_HEADING: &str = "Notes";

const PREAMBLE: &str = "Facts you saved about the user in earlier chats (with `remember`). Use \
                        them naturally, without reciting them. When one changes, `remember` the \
                        new version with `replaces`; when one is wrong, `forget` it.";

pub struct Memory {
    path: PathBuf,
    /// One change at a time: replies in different chats may remember at once.
    lock: Mutex<()>,
}

/// A fact and where it is in the file.
struct Line {
    heading: String,
    text: String,
    at: usize,
}

impl Memory {
    pub fn new(path: PathBuf) -> Self {
        Self {
            path,
            lock: Mutex::new(()),
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn text(&self) -> String {
        std::fs::read_to_string(&self.path).unwrap_or_default()
    }

    pub fn set_text(&self, text: &str) -> Result<(), String> {
        let _guard = self.lock.lock().unwrap();
        self.write(text)
    }

    pub fn view(&self) -> MemoryView {
        let text = self.text();
        MemoryView {
            path: self.path.display().to_string(),
            facts: facts(&text)
                .into_iter()
                .map(|line| MemoryFact {
                    heading: line.heading,
                    text: line.text,
                })
                .collect(),
            text,
        }
    }

    /// Add `fact` under `heading`, or put it in place of the fact `replaces` names. Returns
    /// what happened, for her.
    pub fn remember(
        &self,
        heading: &str,
        fact: &str,
        replaces: Option<&str>,
    ) -> Result<String, String> {
        let fact = one_line(fact);
        if fact.is_empty() {
            return Err("give the fact to remember".into());
        }
        if let Some(kind) = looks_secret(&fact) {
            return Err(refusal(kind));
        }
        let heading = match heading.trim() {
            "" => HEADINGS[0],
            heading => heading,
        };
        let _guard = self.lock.lock().unwrap();
        let text = self.text();
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        let known = facts(&text);
        if let Some(old) = replaces.filter(|r| !r.trim().is_empty()) {
            let line = find(&known, old)?;
            lines[line.at] = format!("- {fact}");
            self.write(&joined(&lines))?;
            return Ok(format!(
                "Remembered \"{fact}\" in place of \"{}\".",
                line.text
            ));
        }
        if known.iter().any(|line| same(&line.text, &fact)) {
            return Ok(format!("Already remembered: \"{fact}\"."));
        }
        let section = lines
            .iter()
            .position(|line| heading_of(line).is_some_and(|h| h.eq_ignore_ascii_case(heading)));
        match section {
            Some(start) => {
                // After the section's last line that isn't blank.
                let end = lines[start + 1..]
                    .iter()
                    .position(|line| heading_of(line).is_some())
                    .map_or(lines.len(), |n| start + 1 + n);
                let mut at = end;
                while at > start + 1 && lines[at - 1].trim().is_empty() {
                    at -= 1;
                }
                lines.insert(at, format!("- {fact}"));
            }
            None => {
                if lines.is_empty() {
                    lines.push("# Lucy's memory".into());
                }
                if lines.last().is_some_and(|l| !l.trim().is_empty()) {
                    lines.push(String::new());
                }
                lines.push(format!("## {heading}"));
                lines.push(format!("- {fact}"));
            }
        }
        self.write(&joined(&lines))?;
        Ok(format!("Remembered under {heading}: \"{fact}\"."))
    }

    pub fn forget(&self, fact: &str) -> Result<String, String> {
        let _guard = self.lock.lock().unwrap();
        let text = self.text();
        let known = facts(&text);
        let line = find(&known, fact)?;
        let mut lines: Vec<String> = text.lines().map(str::to_owned).collect();
        lines.remove(line.at);
        self.write(&joined(&lines))?;
        Ok(format!("Forgot \"{}\".", line.text))
    }

    /// Facts matching `query` by keyword, best first.
    pub fn recall(&self, query: &str, limit: usize) -> Vec<MemoryFact> {
        let all = facts(&self.text());
        let scores = keyword_scores(&all, query);
        let mut ranked: Vec<(f64, &Line)> = all
            .iter()
            .zip(scores)
            .filter(|(_, score)| *score > 0.0)
            .map(|(line, score)| (score, line))
            .collect();
        ranked.sort_by(|a, b| b.0.total_cmp(&a.0));
        ranked
            .into_iter()
            .take(limit)
            .map(|(_, line)| MemoryFact {
                heading: line.heading.clone(),
                text: line.text.clone(),
            })
            .collect()
    }

    /// The memory section of a chat's system prompt, or `None` when there's nothing saved.
    /// `query` is the conversation so far (recent messages), used to pick facts once there
    /// are too many for every prompt.
    pub async fn for_prompt(&self, query: &str, embedder: Option<&Embedder>) -> Option<String> {
        let text = self.text();
        let all = facts(&text);
        if all.is_empty() {
            return None;
        }
        if text.len() <= PROMPT_BUDGET {
            let body = text
                .lines()
                .filter(|line| !line.starts_with("# "))
                .collect::<Vec<_>>()
                .join("\n");
            return Some(format!(
                "## Your memory of the user\n\n{PREAMBLE}\n\n{}",
                body.trim()
            ));
        }

        let always: Vec<bool> = all
            .iter()
            .map(|line| ALWAYS.iter().any(|h| h.eq_ignore_ascii_case(&line.heading)))
            .collect();
        let scores = match embedder {
            Some(embedder) => match self.meaning_scores(&all, query, embedder).await {
                Ok(scores) => scores,
                Err(err) => {
                    eprintln!("memory: embeddings failed, using keywords: {err}");
                    keyword_scores(&all, query)
                }
            },
            None => keyword_scores(&all, query),
        };
        let mut chosen = vec![false; all.len()];
        let mut size = 0;
        for (i, line) in all.iter().enumerate() {
            if always[i] {
                chosen[i] = true;
                size += line.text.len() + 3;
            }
        }
        let mut ranked: Vec<usize> = (0..all.len())
            .filter(|&i| !always[i] && scores[i] > 0.0)
            .collect();
        ranked.sort_by(|&a, &b| scores[b].total_cmp(&scores[a]));
        for i in ranked {
            if size + all[i].text.len() + 3 > PROMPT_BUDGET {
                break;
            }
            chosen[i] = true;
            size += all[i].text.len() + 3;
        }

        let mut sections: Vec<(String, Vec<&str>)> = Vec::new();
        for (line, _) in all.iter().zip(&chosen).filter(|(_, c)| **c) {
            match sections.iter_mut().find(|(h, _)| *h == line.heading) {
                Some((_, facts)) => facts.push(&line.text),
                None => sections.push((line.heading.clone(), vec![&line.text])),
            }
        }
        let body: Vec<String> = sections
            .into_iter()
            .map(|(heading, facts)| {
                let facts: Vec<String> = facts.iter().map(|f| format!("- {f}")).collect();
                format!("## {heading}\n{}", facts.join("\n"))
            })
            .collect();
        let hidden = chosen.iter().filter(|c| !**c).count();
        Some(format!(
            "## Your memory of the user\n\n{PREAMBLE} These are the ones that seem relevant; \
             {hidden} more are saved, so use `recall` to search them when the user mentions \
             something you might know.\n\n{}",
            body.join("\n\n")
        ))
    }

    /// How close each fact is to `query` in meaning. Facts' vectors are kept in
    /// memory.embeddings.json, so each is embedded once per model.
    async fn meaning_scores(
        &self,
        all: &[Line],
        query: &str,
        embedder: &Embedder,
    ) -> Result<Vec<f64>, String> {
        let cache_path = self.path.with_extension("embeddings.json");
        let mut cache: EmbeddingCache = std::fs::read_to_string(&cache_path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .filter(|cache: &EmbeddingCache| cache.model == embedder.model())
            .unwrap_or_else(|| EmbeddingCache {
                model: embedder.model().to_owned(),
                vectors: HashMap::new(),
            });
        let keys: Vec<String> = all.iter().map(|line| key(&line.text)).collect();
        // In the file's order, each fact once.
        let mut seen = HashSet::new();
        let missing: Vec<(String, String)> = all
            .iter()
            .zip(&keys)
            .filter(|(_, k)| !cache.vectors.contains_key(*k) && seen.insert(*k))
            .map(|(line, k)| (k.clone(), line.text.clone()))
            .collect();
        if !missing.is_empty() {
            let texts: Vec<String> = missing.iter().map(|(_, text)| text.clone()).collect();
            let vectors = embedder.embed(&texts).await.map_err(|e| e.to_string())?;
            for ((k, _), vector) in missing.into_iter().zip(vectors) {
                cache.vectors.insert(k, vector);
            }
            // Keep only the facts that still exist.
            cache.vectors.retain(|k, _| keys.contains(k));
            if let Ok(text) = serde_json::to_string(&cache) {
                let _ = std::fs::write(&cache_path, text);
            }
        }
        let wanted = embedder
            .embed(&[query.to_owned()])
            .await
            .map_err(|e| e.to_string())?
            .pop()
            .ok_or("no embedding for the conversation")?;
        Ok(keys
            .iter()
            .map(|k| {
                cache
                    .vectors
                    .get(k)
                    .map_or(0.0, |v| f64::from(cosine(v, &wanted)).max(0.0))
            })
            .collect())
    }

    fn write(&self, text: &str) -> Result<(), String> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).map_err(|err| format!("couldn't save memory: {err}"))?;
        }
        // Write then rename, so a crash never leaves half a file.
        let temp = self.path.with_extension("md.tmp");
        std::fs::write(&temp, text).map_err(|err| format!("couldn't save memory: {err}"))?;
        std::fs::rename(&temp, &self.path).map_err(|err| format!("couldn't save memory: {err}"))
    }
}

#[derive(Serialize, Deserialize)]
struct EmbeddingCache {
    model: String,
    vectors: HashMap<String, Vec<f32>>,
}

fn key(fact: &str) -> String {
    Sha256::digest(fact.as_bytes())
        .iter()
        .take(12)
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn cosine(a: &[f32], b: &[f32]) -> f32 {
    let dot: f32 = a.iter().zip(b).map(|(x, y)| x * y).sum();
    let norm = |v: &[f32]| v.iter().map(|x| x * x).sum::<f32>().sqrt();
    let denominator = norm(a) * norm(b);
    if denominator == 0.0 {
        0.0
    } else {
        dot / denominator
    }
}

/// The facts in memory.md: `- ` (or `* `) lines, with the `##` heading above them.
fn facts(text: &str) -> Vec<Line> {
    let mut heading = NO_HEADING.to_owned();
    let mut out = Vec::new();
    for (at, line) in text.lines().enumerate() {
        if let Some(h) = heading_of(line) {
            heading = h.to_owned();
        } else if let Some(fact) = line
            .trim_start()
            .strip_prefix("- ")
            .or_else(|| line.trim_start().strip_prefix("* "))
        {
            let fact = fact.trim();
            if !fact.is_empty() {
                out.push(Line {
                    heading: heading.clone(),
                    text: fact.to_owned(),
                    at,
                });
            }
        }
    }
    out
}

fn heading_of(line: &str) -> Option<&str> {
    line.strip_prefix("## ").map(str::trim)
}

fn joined(lines: &[String]) -> String {
    let mut text = lines.join("\n");
    text.push('\n');
    text
}

fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn normal(text: &str) -> String {
    one_line(text).trim_end_matches('.').trim().to_lowercase()
}

fn same(a: &str, b: &str) -> bool {
    normal(a) == normal(b)
}

/// The fact `wanted` names: the same text, or the one fact containing it.
fn find<'a>(known: &'a [Line], wanted: &str) -> Result<&'a Line, String> {
    if let Some(line) = known.iter().find(|line| same(&line.text, wanted)) {
        return Ok(line);
    }
    let wanted = normal(wanted);
    let matches: Vec<&Line> = known
        .iter()
        .filter(|line| normal(&line.text).contains(&wanted))
        .collect();
    match matches.as_slice() {
        [one] => Ok(one),
        [] => Err(format!("There's no remembered fact like \"{wanted}\".")),
        many => Err(format!(
            "\"{wanted}\" matches several facts; name one exactly: {}",
            many.iter()
                .map(|line| format!("\"{}\"", line.text))
                .collect::<Vec<_>>()
                .join(", ")
        )),
    }
}

// --- Keyword search ----------------------------------------------------------------------------

const STOPWORDS: &[&str] = &[
    "a", "about", "an", "and", "are", "as", "at", "be", "but", "by", "can", "did", "do", "does",
    "for", "from", "going", "had", "has", "have", "how", "i", "if", "in", "into", "is", "it",
    "its", "me", "my", "of", "on", "or", "our", "s", "she", "so", "that", "the", "their", "them",
    "there", "this", "to", "was", "we", "what", "when", "where", "which", "who", "why", "will",
    "with", "you", "your",
];

/// Lower-case words, without stopwords, with plurals folded ("bikes" → "bike").
fn terms(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .map(str::to_lowercase)
        .filter(|w| !w.is_empty() && !STOPWORDS.contains(&w.as_str()))
        .map(|w| {
            if w.len() > 3 && w.ends_with('s') && !w.ends_with("ss") {
                w[..w.len() - 1].to_owned()
            } else {
                w
            }
        })
        .collect()
}

/// Each fact's score for `query`: the rarer the shared words, the higher.
fn keyword_scores(all: &[Line], query: &str) -> Vec<f64> {
    let wanted: Vec<String> = terms(query);
    let fact_terms: Vec<Vec<String>> = all.iter().map(|line| terms(&line.text)).collect();
    let n = all.len() as f64;
    wanted
        .iter()
        .fold(vec![0.0; all.len()], |mut scores, term| {
            let df = fact_terms.iter().filter(|t| t.contains(term)).count();
            if df > 0 {
                let idf = (1.0 + n / df as f64).ln();
                for (score, t) in scores.iter_mut().zip(&fact_terms) {
                    if t.contains(term) {
                        *score += idf;
                    }
                }
            }
            scores
        })
}

// --- Secrets -----------------------------------------------------------------------------------

static SECRET_PATTERNS: LazyLock<Vec<(Regex, &'static str)>> = LazyLock::new(|| {
    [
        (r"-----BEGIN [A-Z ]*PRIVATE KEY-----", "a private key"),
        (r"\b(sk|rk|pk)-[A-Za-z0-9_-]{16,}", "an API key"),
        (
            r"\b(ghp|gho|ghu|ghs|ghr)_[A-Za-z0-9]{20,}|\bgithub_pat_[A-Za-z0-9_]{20,}",
            "a GitHub token",
        ),
        (r"\bxox[abprs]-[A-Za-z0-9-]{10,}", "a Slack token"),
        (r"\bAKIA[0-9A-Z]{16}\b", "an AWS key"),
        (r"\bAIza[0-9A-Za-z_-]{30,}", "a Google API key"),
        (
            r"(?i)\b(password|passwd|passphrase|passcode|pin)\b\s*(is|:|=)\s*\S+",
            "a password",
        ),
        // A long run of letters and digits mixed: almost always a token or key.
        (r"\b[A-Za-z0-9_\-]{32,}\b", "a token"),
    ]
    .into_iter()
    .map(|(pattern, kind)| (Regex::new(pattern).expect("secret patterns compile"), kind))
    .collect()
});

static CARD: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b\d(?:[ -]?\d){12,18}\b").expect("card pattern compiles"));

/// What kind of secret `text` seems to hold, if any.
pub fn looks_secret(text: &str) -> Option<&'static str> {
    for (pattern, kind) in SECRET_PATTERNS.iter() {
        if let Some(found) = pattern.find(text) {
            // The catch-all only counts runs that mix letters and digits (not long words).
            let token = found.as_str();
            if *kind == "a token"
                && !(token.chars().any(|c| c.is_ascii_digit())
                    && token.chars().any(|c| c.is_ascii_alphabetic()))
            {
                continue;
            }
            return Some(kind);
        }
    }
    CARD.find_iter(text)
        .any(|found| luhn(found.as_str()))
        .then_some("a card number")
}

fn luhn(number: &str) -> bool {
    let digits: Vec<u32> = number.chars().filter_map(|c| c.to_digit(10)).collect();
    if !(13..=19).contains(&digits.len()) {
        return false;
    }
    let sum: u32 = digits
        .iter()
        .rev()
        .enumerate()
        .map(|(i, &d)| {
            if i % 2 == 1 {
                let doubled = d * 2;
                if doubled > 9 { doubled - 9 } else { doubled }
            } else {
                d
            }
        })
        .sum();
    sum.is_multiple_of(10)
}

/// Why a secret wasn't saved, for her.
pub fn refusal(kind: &str) -> String {
    format!(
        "Not saved: that looks like {kind}, and secrets are never kept in memory. Tell the user \
         it wasn't saved; API keys belong in Lucy's keyring (Settings → Models & keys, or `lucy \
         auth set`)."
    )
}
