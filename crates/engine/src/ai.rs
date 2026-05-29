//! AI / model-backed functions, designed around a **refreshable cache**.
//!
//! The engine stays pure and offline: evaluating `AI("prompt")` only *reads* a
//! cached result ([`AiCache`]). Actually contacting a model happens out of band
//! via [`refresh`], which calls a pluggable [`AiProvider`]. Each cached entry
//! carries a timestamp and TTL, so results can be transparently re-fetched when
//! they go stale — no formula edits required.
//!
//! This is the integration seam: a hosted API or a **local model** is wired in
//! simply by supplying an [`AiProvider`] (e.g. an [`FnProvider`] that shells out
//! to a local binary or performs an HTTP call). The core never blocks on I/O.

use crate::formula::Expr;
use crate::value::Value;
use std::collections::HashMap;
use std::time::{Duration, SystemTime};

/// One cached model result with freshness metadata.
#[derive(Debug, Clone)]
pub struct AiEntry {
    pub value: Value,
    pub fetched_at: SystemTime,
    pub ttl: Duration,
}

/// A prompt-keyed cache of model results.
#[derive(Debug, Clone)]
pub struct AiCache {
    entries: HashMap<String, AiEntry>,
    /// TTL applied to entries added via [`AiCache::put`].
    pub default_ttl: Duration,
}

impl Default for AiCache {
    fn default() -> Self {
        AiCache {
            entries: HashMap::new(),
            default_ttl: Duration::from_secs(3600),
        }
    }
}

impl AiCache {
    pub fn new(default_ttl: Duration) -> Self {
        AiCache {
            entries: HashMap::new(),
            default_ttl,
        }
    }

    /// The cached value for a prompt, regardless of freshness.
    pub fn get(&self, prompt: &str) -> Option<Value> {
        self.entries.get(prompt).map(|e| e.value.clone())
    }

    /// Insert/replace a result using the default TTL.
    pub fn put(&mut self, prompt: impl Into<String>, value: Value) {
        let ttl = self.default_ttl;
        self.put_with_ttl(prompt, value, ttl);
    }

    /// Insert/replace a result with an explicit TTL.
    pub fn put_with_ttl(&mut self, prompt: impl Into<String>, value: Value, ttl: Duration) {
        self.entries.insert(
            prompt.into(),
            AiEntry {
                value,
                fetched_at: SystemTime::now(),
                ttl,
            },
        );
    }

    /// Whether a prompt is missing or older than its TTL as of `now`.
    pub fn is_stale(&self, prompt: &str, now: SystemTime) -> bool {
        match self.entries.get(prompt) {
            None => true,
            Some(e) => now
                .duration_since(e.fetched_at)
                .map(|age| age > e.ttl)
                .unwrap_or(true),
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

/// A source of model completions. Implementors may call a hosted API, a local
/// model, or anything else — the engine only ever invokes this during an
/// explicit [`refresh`], never mid-evaluation.
pub trait AiProvider {
    fn complete(&self, prompt: &str) -> Value;
}

/// A provider backed by a closure — the simplest way to wire a real model
/// (HTTP call, local subprocess, etc.) without the engine depending on it.
pub struct FnProvider<F>(pub F);

impl<F: Fn(&str) -> Value> AiProvider for FnProvider<F> {
    fn complete(&self, prompt: &str) -> Value {
        (self.0)(prompt)
    }
}

/// A deterministic provider for tests and demos.
pub struct EchoProvider;

impl AiProvider for EchoProvider {
    fn complete(&self, prompt: &str) -> Value {
        Value::Text(format!("AI[{prompt}]"))
    }
}

/// Re-fetch every stale prompt through `provider`, updating `cache`. Returns the
/// number of entries refreshed. Call this from a background task on whatever
/// cadence suits the app; evaluation then immediately reflects the new values.
pub fn refresh(cache: &mut AiCache, prompts: &[String], provider: &dyn AiProvider) -> usize {
    let now = SystemTime::now();
    let mut refreshed = 0;
    for prompt in prompts {
        if cache.is_stale(prompt, now) {
            cache.put(prompt.clone(), provider.complete(prompt));
            refreshed += 1;
        }
    }
    refreshed
}

/// Collect the literal prompts of `AI(...)` calls in an expression tree (the
/// common case of a constant prompt; dynamic prompts are resolved at eval time).
pub fn collect_prompts(expr: &Expr, out: &mut Vec<String>) {
    match expr {
        Expr::Func(name, args) => {
            if name == "AI" {
                if let Some(Expr::Text(p)) = args.first() {
                    out.push(p.clone());
                }
            }
            for a in args {
                collect_prompts(a, out);
            }
        }
        Expr::Neg(e) | Expr::Percent(e) => collect_prompts(e, out),
        Expr::Binary(_, a, b) => {
            collect_prompts(a, out);
            collect_prompts(b, out);
        }
        Expr::Array(rows) => {
            for e in rows.iter().flatten() {
                collect_prompts(e, out);
            }
        }
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_staleness_by_ttl() {
        let mut cache = AiCache::new(Duration::from_secs(60));
        cache.put("q", Value::Text("a".into()));
        let now = SystemTime::now();
        assert!(!cache.is_stale("q", now));
        // Far in the future, the entry is stale.
        assert!(cache.is_stale("q", now + Duration::from_secs(120)));
        assert!(cache.is_stale("never-fetched", now));
    }

    #[test]
    fn refresh_only_fetches_stale() {
        let mut cache = AiCache::new(Duration::from_secs(3600));
        let prompts = vec!["one".to_string(), "two".to_string()];
        let n = refresh(&mut cache, &prompts, &EchoProvider);
        assert_eq!(n, 2);
        assert_eq!(cache.get("one"), Some(Value::Text("AI[one]".into())));
        // Already fresh → nothing re-fetched.
        assert_eq!(refresh(&mut cache, &prompts, &EchoProvider), 0);
    }

    #[test]
    fn collect_prompts_finds_ai_calls() {
        let ast = crate::formula::parse("AI(\"hi\") & AI(\"there\")").unwrap();
        let mut out = Vec::new();
        collect_prompts(&ast, &mut out);
        assert_eq!(out, vec!["hi".to_string(), "there".to_string()]);
    }
}
