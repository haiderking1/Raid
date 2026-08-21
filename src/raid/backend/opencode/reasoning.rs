use std::collections::HashSet;

use super::error::CatalogError;
use super::types::{
    CatalogDiagnostic, DiagnosticCode, OpenCodeProtocol, ProviderOptions, ReasoningVariant,
    ReasoningVariantKind,
};

#[derive(Debug, Clone, PartialEq)]
pub enum ReasoningOption {
    Effort { values: Vec<Option<String>> },
    Toggle,
    BudgetTokens { min: Option<u64>, max: Option<u64> },
}

#[derive(Debug, Clone, PartialEq)]
enum ReasoningChoice {
    Default,
    Effort { value: String },
    Toggle { enabled: bool },
    Budget { id: &'static str, tokens: u64 },
}

const ANTHROPIC_EFFORTS: &[&str] = &["low", "medium", "high", "xhigh", "max"];
const GOOGLE_LEVELS: &[&str] = &["minimal", "low", "medium", "high"];

pub fn parse_reasoning_options(
    value: Option<&[serde_json::Value]>,
    model_id: &str,
) -> (Vec<ReasoningOption>, Vec<CatalogDiagnostic>) {
    let Some(entries) = value else {
        return (Vec::new(), Vec::new());
    };
    let mut options = Vec::new();
    let mut diagnostics = Vec::new();
    for (index, entry) in entries.iter().enumerate() {
        let Some(kind) = entry.get("type").and_then(|value| value.as_str()) else {
            diagnostics.push(CatalogDiagnostic {
                code: DiagnosticCode::InvalidReasoningOption,
                model_id: Some(model_id.to_string()),
                detail: format!("reasoning_options[{index}] is not a recognized control object."),
            });
            continue;
        };
        match kind {
            "effort" => {
                let Some(values) = entry.get("values").and_then(|value| value.as_array()) else {
                    diagnostics.push(invalid_option(index, kind, model_id));
                    continue;
                };
                let parsed: Vec<Option<String>> = values
                    .iter()
                    .map(|value| {
                        if value.is_null() {
                            None
                        } else {
                            value.as_str().map(str::to_string)
                        }
                    })
                    .collect();
                options.push(ReasoningOption::Effort { values: parsed });
            }
            "toggle" => options.push(ReasoningOption::Toggle),
            "budget_tokens" => {
                let min = entry.get("min").and_then(|value| value.as_u64());
                let max = entry.get("max").and_then(|value| value.as_u64());
                if let (Some(min), Some(max)) = (min, max) {
                    if min > max {
                        diagnostics.push(CatalogDiagnostic {
                            code: DiagnosticCode::InvalidBudgetBounds,
                            model_id: Some(model_id.to_string()),
                            detail: format!(
                                "reasoning_options[{index}] has min {min} greater than max {max}."
                            ),
                        });
                        continue;
                    }
                }
                options.push(ReasoningOption::BudgetTokens { min, max });
            }
            _ => diagnostics.push(CatalogDiagnostic {
                code: DiagnosticCode::UnknownReasoningOption,
                model_id: Some(model_id.to_string()),
                detail: format!(
                    "reasoning_options[{index}] uses unsupported type '{kind}'. Conservative default behavior will be used for that control."
                ),
            }),
        }
    }
    (options, diagnostics)
}

fn invalid_option(index: usize, kind: &str, model_id: &str) -> CatalogDiagnostic {
    CatalogDiagnostic {
        code: DiagnosticCode::InvalidReasoningOption,
        model_id: Some(model_id.to_string()),
        detail: format!("reasoning_options[{index}] has an invalid '{kind}' shape."),
    }
}

pub fn derive_reasoning_variants(
    protocol: OpenCodeProtocol,
    output_limit: u64,
    options: &[ReasoningOption],
    model_id: &str,
) -> Result<(Vec<ReasoningVariant>, Vec<CatalogDiagnostic>), CatalogError> {
    let mut diagnostics = Vec::new();
    let has_effort = options.iter().any(|option| matches!(option, ReasoningOption::Effort { .. }));
    let has_toggle = options.iter().any(|option| matches!(option, ReasoningOption::Toggle));
    let has_budget = options
        .iter()
        .any(|option| matches!(option, ReasoningOption::BudgetTokens { .. }));

    if protocol == OpenCodeProtocol::AnthropicMessages && has_effort && !has_budget {
        diagnostics.push(CatalogDiagnostic {
            code: DiagnosticCode::AmbiguousReasoningMetadata,
            model_id: Some(model_id.to_string()),
            detail: "Catalog advertises Anthropic effort without budget_tokens or a thinking-mode field. Effort is forwarded, but thinking is not enabled because adaptive vs extended thinking cannot be determined from official metadata.".into(),
        });
    }

    if has_toggle
        && matches!(
            protocol,
            OpenCodeProtocol::OpenAiResponses | OpenCodeProtocol::OpenAiCompatible
        )
    {
        diagnostics.push(CatalogDiagnostic {
            code: DiagnosticCode::UnsupportedReasoningControl,
            model_id: Some(model_id.to_string()),
            detail: "Catalog advertises a reasoning toggle, but this protocol has no authoritative thinking wire mapping. The toggle is suppressed.".into(),
        });
    }

    if has_toggle && protocol == OpenCodeProtocol::AnthropicMessages && !has_budget {
        diagnostics.push(CatalogDiagnostic {
            code: DiagnosticCode::UnsupportedReasoningControl,
            model_id: Some(model_id.to_string()),
            detail: "Catalog advertises an Anthropic thinking toggle without budget_tokens. Enabling thinking requires adaptive or extended mode, which official metadata does not distinguish, so the toggle is suppressed.".into(),
        });
    }

    if has_toggle && protocol == OpenCodeProtocol::GoogleGenerativeAi {
        diagnostics.push(CatalogDiagnostic {
            code: DiagnosticCode::UnsupportedReasoningControl,
            model_id: Some(model_id.to_string()),
            detail: "Catalog advertises a Google thinking toggle without a thinking level or token budget. The enabled toggle is suppressed because includeThoughts alone does not enable thinking.".into(),
        });
    }

    if has_budget
        && matches!(
            protocol,
            OpenCodeProtocol::OpenAiResponses | OpenCodeProtocol::OpenAiCompatible
        )
    {
        diagnostics.push(CatalogDiagnostic {
            code: DiagnosticCode::UnsupportedReasoningControl,
            model_id: Some(model_id.to_string()),
            detail: "Catalog advertises budget_tokens, but this protocol has no authoritative token-budget thinking mapping. The budget control is suppressed.".into(),
        });
    }

    let choices = derive_reasoning_choices(options, output_limit);
    let mut variants = Vec::new();
    for choice in choices {
        if let Some(provider_options) = translate_choice(protocol, &choice, options, output_limit) {
            variants.push(variant_from_choice(&choice, provider_options));
        }
    }

    if variants.is_empty() {
        variants.push(default_variant());
    }

    assert_unique_variant_ids(&variants, model_id)?;
    Ok((variants, diagnostics))
}

fn derive_reasoning_choices(options: &[ReasoningOption], output_limit: u64) -> Vec<ReasoningChoice> {
    if options.is_empty() {
        return vec![ReasoningChoice::Default];
    }

    let mut choices = Vec::new();
    for option in options {
        match option {
            ReasoningOption::Effort { values } => {
                for value in values {
                    match value {
                        None => choices.push(ReasoningChoice::Effort {
                            value: "none".into(),
                        }),
                        Some(value) if !value.is_empty() => choices.push(ReasoningChoice::Effort {
                            value: value.clone(),
                        }),
                        _ => {}
                    }
                }
            }
            ReasoningOption::Toggle => {
                choices.push(ReasoningChoice::Toggle { enabled: false });
                choices.push(ReasoningChoice::Toggle { enabled: true });
            }
            ReasoningOption::BudgetTokens { min, max } => {
                choices.extend(budget_choices(*min, *max, output_limit));
            }
        }
    }

    if choices.is_empty() {
        vec![ReasoningChoice::Default]
    } else {
        choices
    }
}

fn budget_choices(min: Option<u64>, max: Option<u64>, output_limit: u64) -> Vec<ReasoningChoice> {
    if output_limit <= 1 {
        return Vec::new();
    }
    let bounded_max = max.unwrap_or(u64::MAX).min(output_limit - 1);
    if bounded_max == 0 {
        return Vec::new();
    }
    let minimum = min.unwrap_or(0);
    if minimum > bounded_max {
        return Vec::new();
    }
    let high = ((bounded_max + 1) / 2).max(minimum).min(bounded_max);
    if high == 0 {
        return Vec::new();
    }
    vec![
        ReasoningChoice::Budget {
            id: "high",
            tokens: high,
        },
        ReasoningChoice::Budget {
            id: "max",
            tokens: bounded_max,
        },
    ]
}

fn translate_choice(
    protocol: OpenCodeProtocol,
    choice: &ReasoningChoice,
    options: &[ReasoningOption],
    output_limit: u64,
) -> Option<ProviderOptions> {
    match choice {
        ReasoningChoice::Default => Some(ProviderOptions::default()),
        ReasoningChoice::Effort { value } => translate_effort(protocol, value, options, output_limit),
        ReasoningChoice::Toggle { enabled } => translate_toggle(protocol, *enabled, options, output_limit),
        ReasoningChoice::Budget { tokens, .. } => translate_budget(protocol, *tokens),
    }
}

fn advertised_budget(options: &[ReasoningOption]) -> Option<&ReasoningOption> {
    options
        .iter()
        .find(|option| matches!(option, ReasoningOption::BudgetTokens { .. }))
}

fn translate_effort(
    protocol: OpenCodeProtocol,
    value: &str,
    options: &[ReasoningOption],
    output_limit: u64,
) -> Option<ProviderOptions> {
    match protocol {
        OpenCodeProtocol::OpenAiResponses => {
            let mut map = serde_json::Map::new();
            map.insert("reasoningEffort".into(), value.into());
            map.insert("forceReasoning".into(), true.into());
            Some(ProviderOptions {
                openai: Some(map),
                ..Default::default()
            })
        }
        OpenCodeProtocol::OpenAiCompatible => Some(ProviderOptions {
            openai_compatible: Some(one_entry("reasoningEffort", value.into())),
            ..Default::default()
        }),
        OpenCodeProtocol::AnthropicMessages => {
            if !ANTHROPIC_EFFORTS.contains(&value) {
                return None;
            }
            let tokens = advertised_budget(options).and_then(|budget| {
                let ReasoningOption::BudgetTokens { min, max } = budget else {
                    return None;
                };
                budget_choices(*min, *max, output_limit)
                    .first()
                    .map(|choice| match choice {
                        ReasoningChoice::Budget { tokens, .. } => *tokens,
                        _ => 0,
                    })
            });
            let mut map = one_entry("effort", value.into());
            if let Some(tokens) = tokens {
                map.insert(
                    "thinking".into(),
                    serde_json::json!({
                        "type": "enabled",
                        "budgetTokens": tokens
                    }),
                );
            }
            Some(ProviderOptions {
                anthropic: Some(map),
                ..Default::default()
            })
        }
        OpenCodeProtocol::GoogleGenerativeAi => {
            if !GOOGLE_LEVELS.contains(&value) {
                return None;
            }
            Some(ProviderOptions {
                google: Some(one_entry(
                    "thinkingConfig",
                    serde_json::json!({
                        "includeThoughts": true,
                        "thinkingLevel": value
                    }),
                )),
                ..Default::default()
            })
        }
    }
}

fn translate_toggle(
    protocol: OpenCodeProtocol,
    enabled: bool,
    options: &[ReasoningOption],
    output_limit: u64,
) -> Option<ProviderOptions> {
    match protocol {
        OpenCodeProtocol::OpenAiResponses | OpenCodeProtocol::OpenAiCompatible => None,
        OpenCodeProtocol::AnthropicMessages => {
            if !enabled {
                if advertised_budget(options).is_none() {
                    return None;
                }
                return Some(ProviderOptions {
                    anthropic: Some(one_entry(
                        "thinking",
                        serde_json::json!({ "type": "disabled" }),
                    )),
                    ..Default::default()
                });
            }
            let tokens = advertised_budget(options).and_then(|budget| {
                let ReasoningOption::BudgetTokens { min, max } = budget else {
                    return None;
                };
                budget_choices(*min, *max, output_limit)
                    .first()
                    .map(|choice| match choice {
                        ReasoningChoice::Budget { tokens, .. } => *tokens,
                        _ => 0,
                    })
            })?;
            Some(ProviderOptions {
                anthropic: Some(one_entry(
                    "thinking",
                    serde_json::json!({
                        "type": "enabled",
                        "budgetTokens": tokens
                    }),
                )),
                ..Default::default()
            })
        }
        OpenCodeProtocol::GoogleGenerativeAi => {
            if enabled {
                return None;
            }
            Some(ProviderOptions {
                google: Some(one_entry(
                    "thinkingConfig",
                    serde_json::json!({ "thinkingBudget": 0 }),
                )),
                ..Default::default()
            })
        }
    }
}

fn translate_budget(protocol: OpenCodeProtocol, tokens: u64) -> Option<ProviderOptions> {
    match protocol {
        OpenCodeProtocol::OpenAiResponses | OpenCodeProtocol::OpenAiCompatible => None,
        OpenCodeProtocol::AnthropicMessages => Some(ProviderOptions {
            anthropic: Some(one_entry(
                "thinking",
                serde_json::json!({
                    "type": "enabled",
                    "budgetTokens": tokens
                }),
            )),
            ..Default::default()
        }),
        OpenCodeProtocol::GoogleGenerativeAi => Some(ProviderOptions {
            google: Some(one_entry(
                "thinkingConfig",
                serde_json::json!({
                    "includeThoughts": true,
                    "thinkingBudget": tokens
                }),
            )),
            ..Default::default()
        }),
    }
}

fn default_variant() -> ReasoningVariant {
    ReasoningVariant {
        id: "default".into(),
        label: "Default".into(),
        kind: ReasoningVariantKind::Default,
        provider_options: ProviderOptions::default(),
    }
}

fn variant_from_choice(choice: &ReasoningChoice, provider_options: ProviderOptions) -> ReasoningVariant {
    match choice {
        ReasoningChoice::Default => default_variant(),
        ReasoningChoice::Effort { value } => ReasoningVariant {
            id: format!("effort:{value}"),
            label: value.clone(),
            kind: ReasoningVariantKind::Effort,
            provider_options,
        },
        ReasoningChoice::Toggle { enabled } => ReasoningVariant {
            id: if *enabled {
                "toggle:enabled".into()
            } else {
                "toggle:disabled".into()
            },
            label: if *enabled { "enabled".into() } else { "disabled".into() },
            kind: ReasoningVariantKind::Toggle,
            provider_options,
        },
        ReasoningChoice::Budget { id, tokens: _ } => ReasoningVariant {
            id: format!("budget:{id}"),
            label: (*id).into(),
            kind: ReasoningVariantKind::Budget,
            provider_options,
        },
    }
}

fn assert_unique_variant_ids(
    variants: &[ReasoningVariant],
    model_id: &str,
) -> Result<(), CatalogError> {
    let mut seen = HashSet::new();
    for variant in variants {
        if !seen.insert(variant.id.clone()) {
            return Err(CatalogError::new(
                "duplicate-reasoning-variant-id",
                format!(
                    "Reasoning variant id '{}' is duplicated for model '{model_id}'.",
                    variant.id
                ),
            ));
        }
    }
    Ok(())
}

fn one_entry(key: &str, value: serde_json::Value) -> serde_json::Map<String, serde_json::Value> {
    let mut map = serde_json::Map::new();
    map.insert(key.into(), value);
    map
}
