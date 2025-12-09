use crate::{
    config::{AIConfig, ModelTierConfig, TaskRouting},
    error::{ApiError, Result},
    models::ai::{
        AITextContinueMode, AITextEditMode, EditInput, EditParams, GeneratedImage,
        GenerationParams, ImageParams, ImageStoryContext, NodeContext, NodeSummary,
        NodeToSummarize, PathNode, StoryContext, StoryContextSimple, TextCandidate,
        TextEditCandidate,
    },
};
use entity::sea_orm_active_enums::AccountTier;
use reqwest::StatusCode;
use serde::{Deserialize, Serialize};
use tracing::{info, instrument};
use uuid::Uuid;

pub struct AIService {
    config: AIConfig,
    http_client: reqwest::Client,
}

#[derive(Debug, Serialize)]
struct OpenAIRequest {
    model: String,
    messages: Vec<OpenAIMessage>,
    max_tokens: u32,
    temperature: f32,
    n: u8,
}

#[derive(Debug, Serialize)]
struct OpenAIMessage {
    role: String,
    content: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponse {
    choices: Vec<OpenAIChoice>,
}

#[derive(Debug, Deserialize)]
struct OpenAIChoice {
    message: OpenAIResponseMessage,
}

#[derive(Debug, Deserialize)]
struct OpenAIResponseMessage {
    content: String,
}

#[derive(Debug, Serialize)]
struct OpenAIImageRequest {
    model: String,
    prompt: String,
    n: u8,
    size: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIImageResponse {
    data: Vec<OpenAIImageData>,
}

#[derive(Debug, Deserialize)]
struct OpenAIImageData {
    url: String,
}

impl AIService {
    pub fn new(config: &AIConfig) -> Self {
        let http_client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_millis(
                config.openrouter.request_timeout_ms,
            ))
            .connect_timeout(std::time::Duration::from_secs(10)) // 10s connection timeout
            .build()
            .expect("Failed to build HTTP client");

        Self {
            config: config.clone(),
            http_client,
        }
    }

    /// Generate text continuations via OpenRouter
    #[instrument(skip(self, context, nodes, account_tier))]
    pub async fn generate_text_continuations(
        &self,
        mode: AITextContinueMode,
        context: &StoryContext,
        nodes: &[PathNode],
        params: &GenerationParams,
        instructions: Option<&str>,
        account_tier: &AccountTier,
    ) -> Result<Vec<TextCandidate>> {
        // Build the prompt based on mode
        let (system_prompt, mut user_prompt) = match mode {
            AITextContinueMode::Prose => {
                let system = format!(
                    "You are a creative writing assistant for Talevonia, a branching narrative app. \
                    Generate {} distinctly different story continuations that could become separate branches. \
                    Each continuation should present a different plot direction, character choice, or narrative possibility.\n\n\
                    For each continuation, provide:\n\
                    Summary: [one-line summary, max 50 characters]\n\
                    Content: [full prose continuation]",
                    params.num_candidates
                );
                let user = self.build_text_prompt(context, nodes, params);
                (system, user)
            }
            AITextContinueMode::Ideas => {
                let system = format!(
                    "You are a creative writing assistant for Talevonia, a branching narrative app. \
                    Generate {} high-level story continuation ideas that represent different branching paths. \
                    Each idea should be a distinct narrative direction (character decision, plot twist, or setting change).\n\n\
                    Format each idea as:\n\
                    Title: [4-12 word summary]\n\
                    [10-30 word description of this branch direction]\n\n\
                    Example:\n\
                    Title: Character enters the mysterious portal\n\
                    A brave but risky choice leading to an unknown realm and new challenges.",
                    params.num_candidates
                );
                let user = self.build_ideas_prompt(context, nodes, params);
                (system, user)
            }
        };

        if let Some(instr) = instructions {
            user_prompt.push_str("\n\nAdditional guidance:\n");
            user_prompt.push_str(instr);
        }

        // Choose model based on routing + size
        let input_chars = user_prompt.len() + system_prompt.len();
        let model = self.select_model(TaskKind::from(mode), account_tier, input_chars)?;

        // Prepare request
        let request = OpenAIRequest {
            model: model.model.clone(),
            messages: vec![
                OpenAIMessage {
                    role: "system".to_string(),
                    content: system_prompt,
                },
                OpenAIMessage {
                    role: "user".to_string(),
                    content: user_prompt,
                },
            ],
            max_tokens: (params.max_words * 2) as u32, // Rough token estimate
            temperature: 0.7,
            n: params.num_candidates,
        };

        // Call OpenRouter API
        let mut attempts = 0;
        let mut last_err = None;
        while attempts <= self.config.openrouter.retry_attempts {
            let mut builder = self
                .http_client
                .post(format!(
                    "{}/chat/completions",
                    self.config.openrouter.api_base
                ))
                .header(
                    "Authorization",
                    format!("Bearer {}", self.config.openrouter.api_key),
                );

            if let Some(ref referer) = self.config.openrouter.referer {
                builder = builder.header("HTTP-Referer", referer);
            }
            if let Some(ref title) = self.config.openrouter.app_title {
                builder = builder.header("X-Title", title);
            }

            let response = builder.json(&request).send().await;

            match response {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
                            attempts += 1;
                            last_err =
                                Some(format!("OpenRouter error {}: {}", status.as_u16(), text));
                            tokio::time::sleep(std::time::Duration::from_millis(
                                200 * attempts as u64,
                            ))
                            .await;
                            continue;
                        }
                        return Err(ApiError::AIProvider(format!(
                            "OpenRouter error {}: {}",
                            status.as_u16(),
                            text
                        )));
                    }

                    let openai_response: OpenAIResponse = resp.json().await.map_err(|e| {
                        ApiError::AIProvider(format!("Failed to parse response: {}", e))
                    })?;

                    info!(
                        "AI vendor response: model={}, choices={}, mode={:?}",
                        model.model,
                        openai_response.choices.len(),
                        mode
                    );

                    // Log first choice content for debugging (truncated)
                    if let Some(first_choice) = openai_response.choices.first() {
                        let preview: String = first_choice.message.content.chars().collect();
                        info!(
                            "AI vendor response preview (mode={:?}): {}...",
                            mode, preview
                        );
                    }

                    let candidates = openai_response
                        .choices
                        .into_iter()
                        .enumerate()
                        .flat_map(|(choice_idx, choice)| {
                            let content = choice.message.content;

                            if mode == AITextContinueMode::Ideas {
                                let (title, parsed_content) = self.parse_idea_response(&content);
                                info!(
                                    "Parsed idea (choice {}): title={:?}, content_len={}",
                                    choice_idx,
                                    title,
                                    parsed_content.len()
                                );
                                vec![TextCandidate {
                                    id: Uuid::new_v4().to_string(),
                                    content: parsed_content,
                                    title,
                                    safety_flags: vec![],
                                }]
                            } else {
                                // Try to parse "Summary: xxx\nContent: yyy" format first
                                let (summary, body) = self.parse_summary_content(&content);
                                if let (Some(s), Some(b)) = (summary, body) {
                                    info!(
                                        "Parsed structured format (choice {}): summary=\"{}\", content_len={}",
                                        choice_idx, s, b.len()
                                    );
                                    vec![TextCandidate {
                                        id: Uuid::new_v4().to_string(),
                                        content: b,
                                        title: Some(s),
                                        safety_flags: vec![],
                                    }]
                                } else {
                                    // Fall back to splitting markdown candidates
                                    info!(
                                        "Structured format not found (choice {}), trying markdown split",
                                        choice_idx
                                    );
                                    let splits = self.split_markdown_candidates(&content);
                                    if splits.len() > 1 {
                                        info!(
                                            "Split into {} markdown sections (choice {})",
                                            splits.len(),
                                            choice_idx
                                        );
                                        splits
                                            .into_iter()
                                            .map(|(title, body)| TextCandidate {
                                                id: Uuid::new_v4().to_string(),
                                                content: body,
                                                title,
                                                safety_flags: vec![],
                                            })
                                            .collect()
                                    } else {
                                        info!(
                                            "No structure detected (choice {}), using raw content with derived title",
                                            choice_idx
                                        );
                                        let parsed = content.trim().to_string();
                                        let title = self.derive_title_from_content(&parsed);
                                        vec![TextCandidate {
                                            id: Uuid::new_v4().to_string(),
                                            content: parsed,
                                            title,
                                            safety_flags: vec![],
                                        }]
                                    }
                                }
                            }
                        })
                        .collect::<Vec<_>>();

                    info!(
                        "Generated {} text continuations in mode {:?} using model {} (downgraded={}, attempts={})",
                        params.num_candidates,
                        mode,
                        model.model,
                        model.downgraded,
                        attempts
                    );

                    return Ok(candidates);
                }
                Err(e) => {
                    attempts += 1;
                    last_err = Some(format!("OpenRouter request failed: {}", e));
                    tokio::time::sleep(std::time::Duration::from_millis(200 * attempts as u64))
                        .await;
                }
            }
        }

        Err(ApiError::AIProvider(
            last_err.unwrap_or_else(|| "OpenRouter request failed".to_string()),
        ))
    }

    /// Generate image using OpenAI DALL-E
    #[instrument(skip(self, context, node))]
    pub async fn generate_image(
        &self,
        context: &ImageStoryContext,
        node: &NodeContext,
        params: &ImageParams,
    ) -> Result<GeneratedImage> {
        let openai_key = self
            .config
            .openai_api_key
            .as_ref()
            .ok_or_else(|| ApiError::AIProvider("OpenAI API key not configured".to_string()))?;
        // Build image prompt
        let prompt = self.build_image_prompt(context, node, params);

        // Prepare size parameter
        let size = match params.aspect_ratio.as_str() {
            "16:9" => "1792x1024",
            "3:4" => "1024x1792",
            _ => "1024x1024",
        };

        let request = OpenAIImageRequest {
            model: "dall-e-3".to_string(),
            prompt: prompt.clone(),
            n: 1,
            size: size.to_string(),
        };

        info!(
            "Generating image with DALL-E 3: size={}, prompt_len={}",
            size,
            prompt.len()
        );

        // Call OpenAI API
        let response = self
            .http_client
            .post("https://api.openai.com/v1/images/generations")
            .header("Authorization", format!("Bearer {}", openai_key))
            .json(&request)
            .send()
            .await
            .map_err(|e| ApiError::AIProvider(format!("OpenAI image request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(ApiError::AIProvider(format!(
                "OpenAI image API error: {}",
                error_text
            )));
        }

        let image_response: OpenAIImageResponse = response
            .json()
            .await
            .map_err(|e| ApiError::AIProvider(format!("Failed to parse image response: {}", e)))?;

        info!(
            "OpenAI image response: {} images returned",
            image_response.data.len()
        );

        let image_url = image_response
            .data
            .first()
            .ok_or_else(|| ApiError::AIProvider("No image generated".to_string()))?
            .url
            .clone();

        // Parse size dimensions
        let (width, height) = if size.contains('x') {
            let parts: Vec<&str> = size.split('x').collect();
            (
                parts[0].parse().unwrap_or(1024),
                parts[1].parse().unwrap_or(1024),
            )
        } else {
            (1024, 1024)
        };

        info!(
            "Generated image with DALL-E 3: url_len={}, {}x{}",
            image_url.len(),
            width,
            height
        );

        Ok(GeneratedImage {
            url: image_url,
            mime_type: "image/png".to_string(),
            width,
            height,
        })
    }

    /// Build prompt for text generation
    fn build_text_prompt(
        &self,
        context: &StoryContext,
        nodes: &[PathNode],
        params: &GenerationParams,
    ) -> String {
        let mut prompt = String::new();

        prompt.push_str(&format!(
            "Story: {}\n",
            context.title.as_deref().unwrap_or("Untitled")
        ));
        prompt.push_str(&format!("Language: {}\n", context.language));

        if !context.tags.is_empty() {
            prompt.push_str(&format!("Genre/Tags: {}\n", context.tags.join(", ")));
        }
        prompt.push_str("\n");

        // Optimize context: use summaries for earlier nodes, full content for recent ones
        if nodes.len() > 5 {
            let split_at = nodes.len() - 3;
            let (earlier, recent) = nodes.split_at(split_at);

            if !earlier.is_empty() {
                prompt.push_str("Story so far:\n");
                for (i, node) in earlier.iter().enumerate() {
                    if let Some(summary) = &node.summary {
                        if !summary.is_empty() {
                            prompt.push_str(&format!("{}. {}\n", i + 1, summary));
                        }
                    } else if !node.content.is_empty() {
                        // Fallback: use first 100 chars
                        let preview: String = node.content.chars().take(100).collect();
                        prompt.push_str(&format!("{}. {}...\n", i + 1, preview));
                    }
                    // Skip nodes with both empty summary and content (chapter nodes)
                }
                prompt.push_str("\n");
            }

            prompt.push_str("Recent events:\n");
            for (i, node) in recent.iter().enumerate() {
                if !node.content.is_empty() {
                    prompt.push_str(&format!("{}. {}\n", split_at + i + 1, node.content));
                } else if let Some(summary) = &node.summary {
                    if !summary.is_empty() {
                        prompt.push_str(&format!("{}. {}\n", split_at + i + 1, summary));
                    }
                }
            }
        } else {
            prompt.push_str("Story so far:\n");
            for (i, node) in nodes.iter().enumerate() {
                if !node.content.is_empty() {
                    prompt.push_str(&format!("{}. {}\n", i + 1, node.content));
                } else if let Some(summary) = &node.summary {
                    if !summary.is_empty() {
                        prompt.push_str(&format!("{}. {}\n", i + 1, summary));
                    }
                }
            }
        }

        prompt.push_str(&format!(
            "\n\nGenerate {} different continuations ({}-{} words each). Each should offer a distinct narrative direction that could become a separate story branch.",
            params.num_candidates,
            params.min_words,
            params.max_words
        ));

        if let Some(tone) = &params.tone {
            prompt.push_str(&format!(" Tone: {}.", tone));
        }

        if params.avoid_hard_end {
            prompt.push_str(" Keep endings open for further branching.");
        }

        prompt
    }

    /// Build prompt for image generation
    fn build_image_prompt(
        &self,
        context: &ImageStoryContext,
        node: &NodeContext,
        params: &ImageParams,
    ) -> String {
        let mut prompt = String::new();

        if let Some(style) = &params.style {
            prompt.push_str(&format!("{} style illustration: ", style));
        } else {
            prompt.push_str("Storybook illustration: ");
        }

        prompt.push_str(&node.content);

        if !node.tags.is_empty() {
            prompt.push_str(&format!(", featuring: {}", node.tags.join(", ")));
        }

        prompt
    }

    /// Build prompt for ideas mode
    fn build_ideas_prompt(
        &self,
        context: &StoryContext,
        nodes: &[PathNode],
        params: &GenerationParams,
    ) -> String {
        let mut prompt = String::new();

        prompt.push_str(&format!(
            "Story: {}\n",
            context.title.as_deref().unwrap_or("Untitled")
        ));
        prompt.push_str(&format!("Language: {}\n", context.language));

        if !context.tags.is_empty() {
            prompt.push_str(&format!("Genre: {}\n", context.tags.join(", ")));
        }
        prompt.push_str("\n");

        // For ideas, use even more aggressive summarization
        if nodes.len() > 3 {
            let split_at = nodes.len() - 2;
            let (earlier, recent) = nodes.split_at(split_at);

            if !earlier.is_empty() {
                prompt.push_str("Story context:\n");
                for node in earlier {
                    if let Some(summary) = &node.summary {
                        if !summary.is_empty() {
                            prompt.push_str(&format!("- {}\n", summary));
                        }
                    } else if !node.content.is_empty() {
                        let preview: String = node.content.chars().take(80).collect();
                        prompt.push_str(&format!("- {}...\n", preview));
                    }
                    // Skip empty chapter nodes
                }
                prompt.push_str("\n");
            }

            prompt.push_str("Current situation:\n");
            for node in recent {
                if let Some(summary) = &node.summary {
                    if !summary.is_empty() {
                        prompt.push_str(&format!("- {}\n", summary));
                    }
                } else if !node.content.is_empty() {
                    let preview: String = node.content.chars().take(150).collect();
                    prompt.push_str(&format!("- {}\n", preview));
                }
                // Skip empty chapter nodes
            }
        } else {
            prompt.push_str("Story so far:\n");
            for node in nodes {
                if let Some(summary) = &node.summary {
                    if !summary.is_empty() {
                        prompt.push_str(&format!("- {}\n", summary));
                    }
                } else if !node.content.is_empty() {
                    prompt.push_str(&format!("- {}\n", node.content));
                }
                // Skip empty chapter nodes
            }
        }

        prompt.push_str(&format!(
            "\n\nGenerate {} different continuation ideas. Each should suggest a distinct branch the story could take.\n\
            Focus on: character decisions, plot directions, or setting changes.",
            params.num_candidates
        ));

        prompt
    }

    /// Parse idea response to extract title and content
    fn parse_idea_response(&self, response: &str) -> (Option<String>, String) {
        // Try to extract title from "Title: xxx" pattern
        if let Some(title_start) = response.find("Title:") {
            let after_title = &response[title_start + 6..].trim();
            if let Some(newline_pos) = after_title.find('\n') {
                let title = after_title[..newline_pos].trim().to_string();
                let content = after_title[newline_pos..].trim().to_string();
                return (Some(title), content);
            }
        }

        // Fallback: use first line as title if present
        let mut lines = response.lines();
        if let Some(first) = lines.next() {
            let title = first.trim();
            let remaining = lines.collect::<Vec<_>>().join("\n").trim().to_string();
            if !title.is_empty() {
                let content = if remaining.is_empty() {
                    response.trim().to_string()
                } else {
                    remaining
                };
                return (Some(title.to_string()), content);
            }
        }

        // Final fallback: no title found
        (None, response.trim().to_string())
    }

    /// Parse response with "Summary: xxx\nContent: yyy" format
    /// Handles both single and multiple structured continuations in one response
    fn parse_summary_content(&self, response: &str) -> (Option<String>, Option<String>) {
        // First check if there are multiple "Summary:" markers (AI put all continuations in one response)
        let summary_count = response.matches("Summary:").count();

        if summary_count > 1 {
            // Multiple structured responses in one - this shouldn't happen with n>1, but handle it
            info!(
                "AI returned {} continuations in single choice (expected separate choices)",
                summary_count
            );
            // Return None to trigger fallback parsing
            return (None, None);
        }

        let lines: Vec<&str> = response.lines().collect();
        let mut summary = None;
        let mut content_start = 0;

        // Look for "Summary:" line
        for (i, line) in lines.iter().enumerate() {
            let trimmed = line.trim();
            if trimmed.to_lowercase().starts_with("summary:") {
                let sum = trimmed[8..].trim();
                if !sum.is_empty() {
                    summary = Some(sum.to_string());
                }
                content_start = i + 1;
                break;
            }
        }

        if summary.is_some() {
            // Look for "Content:" line
            for (i, line) in lines[content_start..].iter().enumerate() {
                let trimmed = line.trim();
                if trimmed.to_lowercase().starts_with("content:") {
                    let remaining: Vec<&str> = lines[content_start + i + 1..].to_vec();
                    let content = remaining.join("\n").trim().to_string();
                    if !content.is_empty() {
                        return (summary, Some(content));
                    }
                    // If "Content:" line has text on same line
                    let inline = trimmed[8..].trim();
                    if !inline.is_empty() {
                        let mut full = inline.to_string();
                        if !remaining.is_empty() {
                            full.push('\n');
                            full.push_str(&remaining.join("\n"));
                        }
                        return (summary, Some(full.trim().to_string()));
                    }
                }
            }

            // "Summary:" found but no "Content:" - treat rest as content
            let remaining: Vec<&str> = lines[content_start..].to_vec();
            let content = remaining.join("\n").trim().to_string();
            if !content.is_empty() {
                return (summary, Some(content));
            }
        }

        (None, None)
    }

    fn derive_title_from_content(&self, content: &str) -> Option<String> {
        let first_line = content.lines().find(|line| !line.trim().is_empty())?;
        let words: Vec<&str> = first_line.split_whitespace().take(8).collect();
        if words.is_empty() {
            None
        } else {
            Some(words.join(" "))
        }
    }

    /// Split a response containing multiple markdown headings (e.g., "**续写一**") into candidates.
    fn split_markdown_candidates(&self, content: &str) -> Vec<(Option<String>, String)> {
        let mut results = Vec::new();
        let mut current_title: Option<String> = None;
        let mut current_body: Vec<String> = Vec::new();
        let mut seen_heading = false;

        for line in content.lines() {
            let trimmed = line.trim();
            let is_heading =
                trimmed.starts_with("**") && trimmed.ends_with("**") && trimmed.len() > 4;

            if is_heading {
                // flush previous
                if seen_heading && !current_body.is_empty() {
                    let body = current_body.join("\n").trim().to_string();
                    results.push((current_title.clone(), body));
                    current_body.clear();
                }
                let title = trimmed.trim_matches('*').trim().to_string();
                current_title = if title.is_empty() { None } else { Some(title) };
                seen_heading = true;
            } else {
                current_body.push(trimmed.to_string());
            }
        }

        if seen_heading && !current_body.is_empty() {
            let body = current_body.join("\n").trim().to_string();
            results.push((current_title.clone(), body));
        }

        if results.is_empty() {
            vec![(None, content.trim().to_string())]
        } else {
            results
        }
    }

    /// Generate text edit/transformation via OpenRouter
    #[instrument(skip(self, input, params, account_tier))]
    pub async fn generate_text_edit(
        &self,
        mode: AITextEditMode,
        story_context: Option<&StoryContextSimple>,
        input: &EditInput,
        params: &EditParams,
        account_tier: &AccountTier,
    ) -> Result<Vec<TextEditCandidate>> {
        let system_prompt = self.build_edit_system_prompt(mode);
        let user_prompt = self.build_edit_user_prompt(mode, story_context, input, params);

        let input_chars = system_prompt.len() + user_prompt.len();
        let model = self.select_model(TaskKind::from(mode), account_tier, input_chars)?;

        // Prepare request with configurable number of candidates
        let request = OpenAIRequest {
            model: model.model.clone(),
            messages: vec![
                OpenAIMessage {
                    role: "system".to_string(),
                    content: system_prompt,
                },
                OpenAIMessage {
                    role: "user".to_string(),
                    content: user_prompt,
                },
            ],
            max_tokens: 2000,
            temperature: 0.7,
            n: params.num_candidates,
        };

        // Call OpenRouter API
        let mut attempts = 0;
        let mut last_err = None;
        while attempts <= self.config.openrouter.retry_attempts {
            let mut builder = self
                .http_client
                .post(format!(
                    "{}/chat/completions",
                    self.config.openrouter.api_base
                ))
                .header(
                    "Authorization",
                    format!("Bearer {}", self.config.openrouter.api_key),
                );

            if let Some(ref referer) = self.config.openrouter.referer {
                builder = builder.header("HTTP-Referer", referer);
            }
            if let Some(ref title) = self.config.openrouter.app_title {
                builder = builder.header("X-Title", title);
            }

            let response = builder.json(&request).send().await;

            match response {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
                            attempts += 1;
                            last_err = Some(format!(
                                "OpenRouter edit error {}: {}",
                                status.as_u16(),
                                text
                            ));
                            tokio::time::sleep(std::time::Duration::from_millis(
                                200 * attempts as u64,
                            ))
                            .await;
                            continue;
                        }
                        return Err(ApiError::AIProvider(format!(
                            "OpenRouter edit error {}: {}",
                            status.as_u16(),
                            text
                        )));
                    }

                    let openai_response: OpenAIResponse = resp.json().await.map_err(|e| {
                        ApiError::AIProvider(format!("Failed to parse edit response: {}", e))
                    })?;

                    info!(
                        "AI vendor edit response: model={}, choices={}, mode={:?}",
                        model.model,
                        openai_response.choices.len(),
                        mode
                    );

                    // Log first choice content preview
                    if let Some(first_choice) = openai_response.choices.first() {
                        let preview: String =
                            first_choice.message.content.chars().take(150).collect();
                        info!(
                            "AI vendor edit response preview (mode={:?}): {}...",
                            mode, preview
                        );
                    }

                    let candidates = openai_response
                        .choices
                        .into_iter()
                        .enumerate()
                        .map(|(idx, choice)| {
                            info!(
                                "Edit candidate {}: content_len={}",
                                idx,
                                choice.message.content.len()
                            );
                            TextEditCandidate {
                                id: Uuid::new_v4().to_string(),
                                content: choice.message.content,
                                safety_flags: vec![],
                            }
                        })
                        .collect();

                    info!(
                        "Generated {} edit candidates in mode {:?} using model {} (downgraded={}, attempts={})",
                        params.num_candidates,
                        mode,
                        model.model,
                        model.downgraded,
                        attempts
                    );

                    return Ok(candidates);
                }
                Err(e) => {
                    attempts += 1;
                    last_err = Some(format!("OpenRouter edit request failed: {}", e));
                    tokio::time::sleep(std::time::Duration::from_millis(200 * attempts as u64))
                        .await;
                }
            }
        }

        Err(ApiError::AIProvider(last_err.unwrap_or_else(|| {
            "OpenRouter edit request failed".to_string()
        })))
    }

    fn build_edit_system_prompt(&self, mode: AITextEditMode) -> String {
        match mode {
            AITextEditMode::Expand => {
                "You are a writing assistant. Expand the given text with more detail, description, or elaboration while maintaining the original meaning and style.".to_string()
            }
            AITextEditMode::Shorten => {
                "You are a writing assistant. Condense the given text to be more concise while preserving the key meaning and impact.".to_string()
            }
            AITextEditMode::Rewrite => {
                "You are a writing assistant. Rewrite the given text with improved clarity, flow, and expression while maintaining the original intent.".to_string()
            }
            AITextEditMode::FixGrammar => {
                "You are a writing assistant. Fix grammar, spelling, and punctuation errors in the given text while preserving the original style and meaning as much as possible.".to_string()
            }
        }
    }

    fn build_edit_user_prompt(
        &self,
        mode: AITextEditMode,
        story_context: Option<&StoryContextSimple>,
        input: &EditInput,
        params: &EditParams,
    ) -> String {
        let mut prompt = String::new();

        // Add story context if available (for generate_ideas mode mainly)
        if let Some(ctx) = story_context {
            if let Some(title) = &ctx.title {
                prompt.push_str(&format!("Story: {}\n", title));
            }
            if let Some(lang) = &ctx.language {
                prompt.push_str(&format!("Language: {}\n", lang));
            }
            if !ctx.tags.is_empty() {
                prompt.push_str(&format!("Tags: {}\n", ctx.tags.join(", ")));
            }
            if !prompt.is_empty() {
                prompt.push_str("\n");
            }
        }

        // Determine target text (selection or full text)
        let target_text = input.selection.as_ref().unwrap_or(&input.text);

        // Add edit-specific instructions
        match mode {
            AITextEditMode::Expand => {
                if let Some(target_len) = &params.target_length {
                    prompt.push_str(&format!("Target length: {}\n", target_len));
                }
                prompt.push_str("\nText to expand:\n");
                prompt.push_str(target_text);
            }
            AITextEditMode::Shorten => {
                if let Some(target_len) = &params.target_length {
                    prompt.push_str(&format!("Target length: {}\n", target_len));
                }
                prompt.push_str("\nText to shorten:\n");
                prompt.push_str(target_text);
            }
            AITextEditMode::Rewrite => {
                if let Some(tone) = &params.tone {
                    prompt.push_str(&format!("Desired tone: {}\n", tone));
                }
                if params.keep_style.unwrap_or(true) {
                    prompt.push_str("Keep the original writing style.\n");
                }
                prompt.push_str("\nText to rewrite:\n");
                prompt.push_str(target_text);
            }
            AITextEditMode::FixGrammar => {
                prompt.push_str("Text to fix:\n");
                prompt.push_str(target_text);
                prompt.push_str("\n\nReturn only the corrected text without explanations.");
            }
        }

        // Add language override if specified
        if let Some(lang) = &params.language {
            prompt.push_str(&format!("\n\nLanguage: {}", lang));
        }

        prompt
    }

    /// Generate summaries for multiple nodes
    #[instrument(skip(self, _story_context, nodes, account_tier))]
    pub async fn generate_summaries(
        &self,
        _story_context: Option<&StoryContextSimple>,
        nodes: &[NodeToSummarize],
        account_tier: &AccountTier,
    ) -> Result<Vec<NodeSummary>> {
        // Build system prompt
        let system_prompt =
            "You are a writing assistant that generates concise one-line summaries. \
            Each summary should be max 50 characters and capture the key action or event.";

        // Build user prompt
        let mut user_prompt = String::new();
        user_prompt.push_str("Generate a one-line summary (max 50 characters) for each text:\n\n");

        for (i, node) in nodes.iter().enumerate() {
            user_prompt.push_str(&format!("{}. {}\n\n", i + 1, node.content));
        }

        user_prompt.push_str("Format your response as:\n1. [summary]\n2. [summary]\n...");

        let input_chars = system_prompt.len() + user_prompt.len();
        let model = self.select_model(TaskKind::Summarize, account_tier, input_chars)?;

        // Prepare request
        let request = OpenAIRequest {
            model: model.model.clone(),
            messages: vec![
                OpenAIMessage {
                    role: "system".to_string(),
                    content: system_prompt.to_string(),
                },
                OpenAIMessage {
                    role: "user".to_string(),
                    content: user_prompt,
                },
            ],
            max_tokens: 500,
            temperature: 0.5,
            n: 1,
        };

        // Call OpenRouter API
        let mut attempts = 0;
        let mut last_err = None;
        while attempts <= self.config.openrouter.retry_attempts {
            let mut builder = self
                .http_client
                .post(format!(
                    "{}/chat/completions",
                    self.config.openrouter.api_base
                ))
                .header(
                    "Authorization",
                    format!("Bearer {}", self.config.openrouter.api_key),
                );

            if let Some(ref referer) = self.config.openrouter.referer {
                builder = builder.header("HTTP-Referer", referer);
            }
            if let Some(ref title) = self.config.openrouter.app_title {
                builder = builder.header("X-Title", title);
            }

            let response = builder.json(&request).send().await;

            match response {
                Ok(resp) => {
                    if !resp.status().is_success() {
                        let status = resp.status();
                        let text = resp.text().await.unwrap_or_default();
                        if status.is_server_error() || status == StatusCode::TOO_MANY_REQUESTS {
                            attempts += 1;
                            last_err = Some(format!(
                                "OpenRouter summarize error {}: {}",
                                status.as_u16(),
                                text
                            ));
                            tokio::time::sleep(std::time::Duration::from_millis(
                                200 * attempts as u64,
                            ))
                            .await;
                            continue;
                        }
                        return Err(ApiError::AIProvider(format!(
                            "OpenRouter summarize error {}: {}",
                            status.as_u16(),
                            text
                        )));
                    }

                    let openai_response: OpenAIResponse = resp.json().await.map_err(|e| {
                        ApiError::AIProvider(format!("Failed to parse summarize response: {}", e))
                    })?;

                    info!(
                        "AI vendor summarize response: model={}, choices={}, nodes_count={}",
                        model.model,
                        openai_response.choices.len(),
                        nodes.len()
                    );

                    let content = openai_response
                        .choices
                        .first()
                        .map(|c| c.message.content.as_str())
                        .unwrap_or("");

                    // Log raw response
                    info!(
                        "AI vendor summarize raw response: {}",
                        content.chars().take(300).collect::<String>()
                    );

                    // Parse numbered list
                    let summaries = self.parse_numbered_summaries(content, nodes);

                    info!(
                        "Parsed {} summaries from response (expected {})",
                        summaries.len(),
                        nodes.len()
                    );

                    info!(
                        "Generated {} summaries using model {} (downgraded={}, attempts={})",
                        nodes.len(),
                        model.model,
                        model.downgraded,
                        attempts
                    );

                    return Ok(summaries);
                }
                Err(e) => {
                    attempts += 1;
                    last_err = Some(format!("OpenRouter summarize request failed: {}", e));
                    tokio::time::sleep(std::time::Duration::from_millis(200 * attempts as u64))
                        .await;
                }
            }
        }

        Err(ApiError::AIProvider(last_err.unwrap_or_else(|| {
            "OpenRouter summarize request failed".to_string()
        })))
    }

    /// Parse numbered list of summaries
    fn parse_numbered_summaries(
        &self,
        content: &str,
        nodes: &[NodeToSummarize],
    ) -> Vec<NodeSummary> {
        let mut summaries = Vec::new();
        let lines: Vec<&str> = content.lines().collect();

        for (i, node) in nodes.iter().enumerate() {
            let mut found = false;
            let prefix = format!("{}.", i + 1);

            for line in &lines {
                let trimmed = line.trim();
                if trimmed.starts_with(&prefix) {
                    let summary = trimmed[prefix.len()..].trim().to_string();
                    if !summary.is_empty() {
                        summaries.push(NodeSummary {
                            node_id: node.node_id.clone(),
                            summary: summary.chars().take(50).collect(),
                        });
                        found = true;
                        break;
                    }
                }
            }

            if !found {
                // Fallback: generate from content
                let fallback: String = node.content.chars().take(50).collect();
                summaries.push(NodeSummary {
                    node_id: node.node_id.clone(),
                    summary: fallback,
                });
            }
        }

        summaries
    }
}

#[derive(Debug, Clone)]
struct SelectedModel {
    model: String,
    max_context_tokens: u32,
    downgraded: bool,
}

#[derive(Debug, Clone, Copy)]
enum TaskKind {
    FixGrammar,
    Shorten,
    Rewrite,
    Ideas,
    Continue,
    Expand,
    Summarize,
}

impl From<AITextContinueMode> for TaskKind {
    fn from(mode: AITextContinueMode) -> Self {
        match mode {
            AITextContinueMode::Prose => TaskKind::Continue,
            AITextContinueMode::Ideas => TaskKind::Ideas,
        }
    }
}

impl From<AITextEditMode> for TaskKind {
    fn from(mode: AITextEditMode) -> Self {
        match mode {
            AITextEditMode::Expand => TaskKind::Expand,
            AITextEditMode::Shorten => TaskKind::Shorten,
            AITextEditMode::Rewrite => TaskKind::Rewrite,
            AITextEditMode::FixGrammar => TaskKind::FixGrammar,
        }
    }
}

impl AIService {
    fn select_model(
        &self,
        task: TaskKind,
        account_tier: &AccountTier,
        input_chars: usize,
    ) -> Result<SelectedModel> {
        let routing: &TaskRouting = match task {
            TaskKind::FixGrammar => &self.config.openrouter.ai_routing.fix_grammar,
            TaskKind::Shorten => &self.config.openrouter.ai_routing.shorten,
            TaskKind::Rewrite => &self.config.openrouter.ai_routing.rewrite,
            TaskKind::Ideas => &self.config.openrouter.ai_routing.ideas,
            TaskKind::Continue => &self.config.openrouter.ai_routing.r#continue,
            TaskKind::Expand => &self.config.openrouter.ai_routing.expand,
            TaskKind::Summarize => &self.config.openrouter.ai_routing.fix_grammar, // Reuse light task
        };

        let mut tier_name = match account_tier {
            AccountTier::Free => routing.free_default_tier.as_str(),
            AccountTier::Pro => routing.pro_default_tier.as_str(),
        };

        let mut downgraded = false;
        if let Some(threshold) = routing.downgrade_over_chars {
            if input_chars > threshold {
                downgraded = true;
                tier_name = match tier_name {
                    "premium" => "standard",
                    "standard" => "light",
                    other => other,
                };
            }
        }

        let tier_config: &ModelTierConfig = match tier_name {
            "premium" => &self.config.openrouter.model_tiers.premium,
            "standard" => &self.config.openrouter.model_tiers.standard,
            "light" => &self.config.openrouter.model_tiers.light,
            other => {
                return Err(ApiError::AIProvider(format!(
                    "Unknown model tier: {}",
                    other
                )))
            }
        };

        Ok(SelectedModel {
            model: tier_config.model.clone(),
            max_context_tokens: tier_config.max_context_tokens,
            downgraded,
        })
    }
}
