use crate::event::TokenUsage;

struct ModelPricing {
    input_per_m: f64,
    output_per_m: f64,
    cache_read_per_m: f64,
    cache_write_per_m: f64,
}

const CLAUDE_SONNET: ModelPricing = ModelPricing {
    input_per_m: 3.0,
    output_per_m: 15.0,
    cache_read_per_m: 0.30,
    cache_write_per_m: 3.75,
};

const CLAUDE_OPUS: ModelPricing = ModelPricing {
    input_per_m: 15.0,
    output_per_m: 75.0,
    cache_read_per_m: 1.50,
    cache_write_per_m: 18.75,
};

const GPT_4_1: ModelPricing = ModelPricing {
    input_per_m: 2.0,
    output_per_m: 8.0,
    cache_read_per_m: 0.0,
    cache_write_per_m: 0.0,
};

fn pricing_for_model(model: &str) -> &'static ModelPricing {
    let lower = model.to_lowercase();
    if lower.contains("opus") {
        &CLAUDE_OPUS
    } else if lower.contains("sonnet") || lower.contains("claude") {
        &CLAUDE_SONNET
    } else if lower.contains("gpt") {
        &GPT_4_1
    } else {
        &CLAUDE_SONNET
    }
}

pub fn estimate_cost(model: &str, tokens: &TokenUsage) -> f64 {
    let p = pricing_for_model(model);
    let input_cost = (tokens.input as f64 / 1_000_000.0) * p.input_per_m;
    let output_cost = (tokens.output as f64 / 1_000_000.0) * p.output_per_m;
    let cache_read_cost = (tokens.cache_read as f64 / 1_000_000.0) * p.cache_read_per_m;
    let cache_write_cost = (tokens.cache_write as f64 / 1_000_000.0) * p.cache_write_per_m;
    input_cost + output_cost + cache_read_cost + cache_write_cost
}

pub fn format_tokens(n: u64) -> String {
    if n >= 1_000_000 {
        format!("{:.1}M", n as f64 / 1_000_000.0)
    } else if n >= 1_000 {
        format!("{:.1}k", n as f64 / 1_000.0)
    } else {
        format!("{n}")
    }
}

pub fn format_cost(usd: f64) -> String {
    if usd < 0.01 {
        format!("${:.4}", usd)
    } else {
        format!("${:.2}", usd)
    }
}

pub fn cache_hit_rate(tokens: &TokenUsage) -> f64 {
    let total_input = tokens.input + tokens.cache_read;
    if total_input == 0 {
        0.0
    } else {
        (tokens.cache_read as f64 / total_input as f64) * 100.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estimate_cost_claude_sonnet() {
        let tokens = TokenUsage {
            input: 10_000,
            output: 5_000,
            cache_read: 100_000,
            cache_write: 1_000,
        };
        let cost = estimate_cost("claude-sonnet-4-5", &tokens);
        let expected = (10_000.0 / 1e6) * 3.0
            + (5_000.0 / 1e6) * 15.0
            + (100_000.0 / 1e6) * 0.30
            + (1_000.0 / 1e6) * 3.75;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn estimate_cost_claude_opus() {
        let tokens = TokenUsage {
            input: 1_000_000,
            output: 100_000,
            cache_read: 0,
            cache_write: 0,
        };
        let cost = estimate_cost("claude-opus-4.6", &tokens);
        let expected = 15.0 + 7.5;
        assert!((cost - expected).abs() < 1e-10);
    }

    #[test]
    fn format_tokens_small() {
        assert_eq!(format_tokens(500), "500");
    }

    #[test]
    fn format_tokens_thousands() {
        assert_eq!(format_tokens(12_400), "12.4k");
    }

    #[test]
    fn format_tokens_millions() {
        assert_eq!(format_tokens(2_500_000), "2.5M");
    }

    #[test]
    fn format_cost_small() {
        assert_eq!(format_cost(0.0012), "$0.0012");
    }

    #[test]
    fn format_cost_normal() {
        assert_eq!(format_cost(0.45), "$0.45");
    }

    #[test]
    fn format_cost_large() {
        assert_eq!(format_cost(12.34), "$12.34");
    }

    #[test]
    fn cache_hit_rate_no_input() {
        let t = TokenUsage::default();
        assert_eq!(cache_hit_rate(&t), 0.0);
    }

    #[test]
    fn cache_hit_rate_all_cached() {
        let t = TokenUsage {
            input: 0,
            output: 100,
            cache_read: 1000,
            cache_write: 0,
        };
        assert_eq!(cache_hit_rate(&t), 100.0);
    }

    #[test]
    fn cache_hit_rate_partial() {
        let t = TokenUsage {
            input: 500,
            output: 100,
            cache_read: 500,
            cache_write: 0,
        };
        assert_eq!(cache_hit_rate(&t), 50.0);
    }
}
