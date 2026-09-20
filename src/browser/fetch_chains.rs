//! `fetch(url).then(...)` support for the tiny JS runtime.
//!
//! A fetch expression is captured at script time as a [`FetchChain`] and
//! parked until a lane supplies the response body (local files in the fixture
//! lane, real HTTP in hosted sessions). Parked chains settle only after the
//! timers queued ahead of them have run, and each `.then` step then executes
//! as its own microtask, matching the event-loop order Chromium produces.

use super::{
    Dom, TinyJsRuntime, evaluate_js_string_expression, execute_js_statement,
    parse_arrow_first_parameter, parse_function_first_parameter, parse_js_callable_body,
    parse_js_named_call, resolve_browser_href, split_js_arguments, split_js_statements,
};

/// The settled result of one `fetch(url)`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchOutcome {
    pub ok: bool,
    pub status: u16,
    pub body: String,
}

impl FetchOutcome {
    pub fn success(body: String) -> Self {
        Self {
            ok: true,
            status: 200,
            body,
        }
    }

    pub fn failure(status: u16) -> Self {
        Self {
            ok: false,
            status,
            body: String::new(),
        }
    }
}

/// One `.then(callback)` in a fetch chain.
#[derive(Debug, Clone)]
pub(super) struct FetchChainStep {
    parameter: Option<String>,
    body: String,
}

/// A `fetch(url).then(a).then(b)...` expression captured at script time. Each
/// step runs as its own microtask once the response has settled; `value` is
/// the string flowing through the chain (the response body after a
/// `response.text()` / `response.json()` passthrough step).
#[derive(Debug, Clone)]
pub(super) struct FetchChain {
    resolved_url: String,
    steps: Vec<FetchChainStep>,
    next_step: usize,
    value: String,
}

const FETCH_CHAIN_STATEMENT: &str = "__esoteric_fetch_chain__";

/// `fetch(url).then(cb)...` — the URL is resolved against the page and the
/// `.then` steps are captured. If the response is already known the chain is
/// queued as microtasks; otherwise it parks in `pending_fetch_chains` for an
/// async lane to settle. `.catch`/`.finally` steps are accepted and skipped.
pub(super) fn execute_js_fetch(dom: &Dom, runtime: &mut TinyJsRuntime, statement: &str) -> bool {
    let statement = statement.trim();
    let Some(after_name) = statement
        .strip_prefix("window.fetch")
        .or_else(|| statement.strip_prefix("fetch"))
        .map(str::trim_start)
    else {
        return false;
    };
    if !after_name.starts_with('(') {
        return false;
    }
    let Some(close) = find_matching_js_paren(after_name, 0) else {
        return true;
    };
    let args = split_js_arguments(&after_name[1..close]);
    let Some(url) = args
        .first()
        .and_then(|arg| evaluate_js_string_expression(dom, runtime, arg.trim()))
    else {
        return true;
    };
    let resolved_url = resolve_browser_href(&runtime.page_source, url.trim());
    let steps = parse_fetch_chain_steps(&after_name[close + 1..]);
    if steps.is_empty() {
        return true;
    }
    let chain = FetchChain {
        resolved_url,
        steps,
        next_step: 0,
        value: String::new(),
    };
    // Even a known response settles as a later task, never a microtask of
    // the current script: `drain_timer_tasks` resolves parked chains only
    // after the timers queued so far have run, which is the order a real
    // network response arrives in.
    runtime.pending_fetch_chains.push(chain);
    true
}

fn parse_fetch_chain_steps(mut rest: &str) -> Vec<FetchChainStep> {
    let mut steps = Vec::new();
    loop {
        rest = rest.trim_start();
        let (method, keep) = if rest.starts_with(".then") {
            (".then", true)
        } else if rest.starts_with(".catch") {
            (".catch", false)
        } else if rest.starts_with(".finally") {
            (".finally", false)
        } else {
            break;
        };
        let after_method = rest[method.len()..].trim_start();
        if !after_method.starts_with('(') {
            break;
        }
        let Some(close) = find_matching_js_paren(after_method, 0) else {
            break;
        };
        if keep {
            let callback = after_method[1..close].trim();
            let callback = split_js_arguments(callback)
                .first()
                .map(|first| first.trim())
                .unwrap_or(callback);
            if let Some(step) = parse_fetch_chain_step(callback) {
                steps.push(step);
            }
        }
        rest = &after_method[close + 1..];
    }
    steps
}

fn parse_fetch_chain_step(callback: &str) -> Option<FetchChainStep> {
    let callback = callback
        .trim()
        .strip_prefix("async")
        .map(str::trim_start)
        .unwrap_or(callback.trim());
    if callback.starts_with("function") {
        let body = parse_js_callable_body(callback)?;
        return Some(FetchChainStep {
            parameter: parse_function_first_parameter(callback),
            body,
        });
    }
    let arrow_index = callback.find("=>")?;
    let parameter = parse_arrow_first_parameter(&callback[..arrow_index]);
    let body_expression = callback[arrow_index + 2..].trim();
    let body = parse_js_callable_body(body_expression)
        .unwrap_or_else(|| body_expression.trim_end_matches(';').trim().to_owned());
    Some(FetchChainStep { parameter, body })
}

/// Index of the `)` matching the `(` at `open`, skipping string literals and
/// nested brackets of any kind.
fn find_matching_js_paren(text: &str, open: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if bytes.get(open) != Some(&b'(') {
        return None;
    }
    let mut depth = 0usize;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    for (index, &byte) in bytes.iter().enumerate().skip(open) {
        if let Some(active) = quote {
            if escaped {
                escaped = false;
            } else if byte == b'\\' {
                escaped = true;
            } else if byte == active {
                quote = None;
            }
            continue;
        }
        match byte {
            b'"' | b'\'' | b'`' => quote = Some(byte),
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' => {
                depth = depth.saturating_sub(1);
                if depth == 0 {
                    return (byte == b')').then_some(index);
                }
            }
            _ => {}
        }
    }
    None
}

fn queue_fetch_chain(runtime: &mut TinyJsRuntime, mut chain: FetchChain, outcome: &FetchOutcome) {
    // A failed response rejects the promise; without `.catch` support the
    // chain simply never runs, which is what an unhandled rejection looks like.
    if !outcome.ok {
        return;
    }
    chain.value = outcome.body.clone();
    let id = runtime.next_fetch_chain_id;
    runtime.next_fetch_chain_id = runtime.next_fetch_chain_id.saturating_add(1).max(1);
    runtime.fetch_chains.insert(id, chain);
    runtime
        .microtask_queue
        .push_back(format!("{FETCH_CHAIN_STATEMENT}({id})"));
}

/// Called by an async lane after it has stored responses for every URL the
/// parked chains were waiting on; chains whose URL is still unknown stay
/// parked. Returns how many chains were queued.
pub(super) fn resolve_pending_fetch_chains(runtime: &mut TinyJsRuntime) -> usize {
    let pending = std::mem::take(&mut runtime.pending_fetch_chains);
    let mut queued = 0usize;
    for chain in pending {
        match runtime.fetch_results.get(&chain.resolved_url).cloned() {
            Some(outcome) => {
                queue_fetch_chain(runtime, chain, &outcome);
                queued += 1;
            }
            None => runtime.pending_fetch_chains.push(chain),
        }
    }
    queued
}

/// URLs the parked chains are waiting on, deduplicated in first-seen order.
pub(super) fn pending_fetch_urls(runtime: &TinyJsRuntime) -> Vec<String> {
    let mut urls = Vec::new();
    for chain in &runtime.pending_fetch_chains {
        if !urls.contains(&chain.resolved_url) {
            urls.push(chain.resolved_url.clone());
        }
    }
    urls
}

/// Runs one `.then` step of a chain as a microtask and queues the next one.
pub(super) fn execute_js_fetch_chain_step(
    dom: &mut Dom,
    runtime: &mut TinyJsRuntime,
    statement: &str,
) -> bool {
    let Some(args) = parse_js_named_call(statement, &[FETCH_CHAIN_STATEMENT]) else {
        return false;
    };
    let Some(id) = args.trim().parse::<u64>().ok() else {
        return true;
    };
    let Some(mut chain) = runtime.fetch_chains.remove(&id) else {
        return true;
    };
    let Some(step) = chain.steps.get(chain.next_step).cloned() else {
        return true;
    };
    let body = step.body.trim().trim_end_matches(';').trim();
    let expression = body.strip_prefix("return ").unwrap_or(body).trim();
    let passthrough = step.parameter.as_deref().is_some_and(|parameter| {
        expression == format!("{parameter}.text()") || expression == format!("{parameter}.json()")
    });
    if !passthrough {
        let previous = step.parameter.as_ref().map(|parameter| {
            runtime
                .string_bindings
                .insert(parameter.clone(), chain.value.clone())
        });
        let previous_this = runtime.this_node.take();
        let previous_this_target = runtime.this_target.take();
        for statement in split_js_statements(&step.body) {
            execute_js_statement(dom, runtime, statement);
        }
        runtime.this_node = previous_this;
        runtime.this_target = previous_this_target;
        if let Some(parameter) = &step.parameter {
            match previous.flatten() {
                Some(value) => {
                    runtime.string_bindings.insert(parameter.clone(), value);
                }
                None => {
                    runtime.string_bindings.remove(parameter);
                }
            }
        }
    }
    chain.next_step += 1;
    if chain.next_step < chain.steps.len() {
        runtime.fetch_chains.insert(id, chain);
        runtime
            .microtask_queue
            .push_back(format!("{FETCH_CHAIN_STATEMENT}({id})"));
    }
    true
}
