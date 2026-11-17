//! The one price table for the hosted model provider: what the worker's daily cap charges (`pk_llm::budget`) and what
//! the usage meter prices every recorded call at (pk-api `usage`), so the two never disagree. First-party Anthropic
//! list prices per million tokens; [`PRICES_READ`] and [`PRICES_SOURCE`] say when and where they were read. Re-read
//! the page and move both (and `engine/budget.py`, the Python oracle the budget golden is generated from) when the
//! prices change.

/// USD per million tokens: input, output, cache write (5-minute), cache read.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Prices {
    /// Uncached input.
    pub input: f64,
    /// Output.
    pub output: f64,
    /// Cache write.
    pub cache_write: f64,
    /// Cache read.
    pub cache_read: f64,
}

const fn p(input: f64, output: f64, cache_write: f64, cache_read: f64) -> Prices {
    Prices {
        input,
        output,
        cache_write,
        cache_read,
    }
}

/// The day [`PRICES`] was read off [`PRICES_SOURCE`].
pub const PRICES_READ: &str = "2026-11-17";
/// Where [`PRICES`] was read: Anthropic's pricing page (first-party API, global routing, 5-minute cache writes,
/// no batch).
pub const PRICES_SOURCE: &str = "https://platform.claude.com/docs/en/about-claude/pricing";

/// Model id prefix → prices (first-party list prices, read [`PRICES_READ`]); the longest matching prefix wins.
/// Cache writes are the 5-minute ones: the worker's `cache_control` sets no `ttl`.
pub const PRICES: &[(&str, Prices)] = &[
    ("claude-opus-5-5", p(4.00, 20.00, 5.00, 0.20)),
    // cache hits on Sonnet 5.5 are 0.05x input: $0.10 (the table said $0.20 until 2026-11-17)
    ("claude-sonnet-5-5", p(2.00, 10.00, 2.50, 0.10)),
    // prompts up to 100,000 tokens; longer prompts pay LONG_PRICES
    ("claude-haiku-5-5", p(0.10, 0.50, 0.125, 0.01)),
    ("claude-opus-5", p(5.00, 25.00, 6.25, 0.50)),
    ("claude-sonnet-5", p(2.00, 10.00, 2.50, 0.20)),
    ("claude-haiku-4-5", p(1.00, 5.00, 1.25, 0.10)),
    ("claude-fable-5-2", p(8.00, 40.00, 10.00, 0.20)),
    ("claude-fable-5-1", p(10.00, 50.00, 12.50, 0.25)),
];

/// Models priced by prompt length: above the threshold (every input token of the request: uncached, cache writes and
/// cache reads) the whole request pays the second set of prices ("Long context pricing" on [`PRICES_SOURCE`]).
pub const LONG_PRICES: &[(&str, u64, Prices)] = &[("claude-haiku-5-5", 100_000, p(0.50, 2.50, 0.625, 0.05))];

/// The price of a model not in the table: the most expensive one in it (Fable 5.1), so a mistake can only
/// over-count.
pub const FALLBACK_PRICE: Prices = PRICES[7].1;

/// `budget.py` `price_of`: case-insensitive prefix match, the longest prefix wins, [`FALLBACK_PRICE`] when none
/// matches.
pub fn price_of(model: &str) -> Prices {
    let m = model.to_lowercase();
    PRICES
        .iter()
        .filter(|(k, _)| m.starts_with(k))
        .max_by_key(|(k, _)| k.len())
        .map_or(FALLBACK_PRICE, |(_, v)| *v)
}

/// The prices one request pays: [`price_of`], or the model's long-prompt prices when its prompt (`prompt_tokens`,
/// every input token) is over the threshold. The long rule follows the same prefix the short price matched.
pub fn price_for(model: &str, prompt_tokens: u64) -> Prices {
    let m = model.to_lowercase();
    let short = PRICES
        .iter()
        .filter(|(k, _)| m.starts_with(k))
        .max_by_key(|(k, _)| k.len());
    let Some((key, base)) = short else {
        return FALLBACK_PRICE;
    };
    LONG_PRICES
        .iter()
        .find(|(k, over, _)| k == key && prompt_tokens > *over)
        .map_or(*base, |(_, _, long)| *long)
}

/// USD for one request from its token counts (the usage meter's call rows carry exactly these four).
pub fn tokens_usd(model: &str, input: u64, output: u64, cache_write: u64, cache_read: u64) -> f64 {
    let pr = price_for(model, input + cache_write + cache_read);
    (input as f64 * pr.input
        + output as f64 * pr.output
        + cache_write as f64 * pr.cache_write
        + cache_read as f64 * pr.cache_read)
        / 1_000_000.0
}
