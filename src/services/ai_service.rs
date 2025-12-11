use crate::{
    config::{AIConfig, ModelTierConfig, TaskRouting},
    error::{ApiError, Result},
    models::ai::{
        AITextEditMode, Background, Character, EditInput, EditParams, GeneratedImage,
        GenerationParams, ImageParams, ImageStoryContext, ImageStyle, NodeContext, NodeSummary,
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
    #[serde(skip_serializing_if = "Option::is_none")]
    response_format: Option<ResponseFormat>,
}

#[derive(Debug, Serialize)]
struct ResponseFormat {
    #[serde(rename = "type")]
    format_type: String,
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
    quality: String,
}

#[derive(Debug, Deserialize)]
struct OpenAIImageResponse {
    data: Vec<OpenAIImageData>,
}

#[derive(Debug, Deserialize)]
struct OpenAIImageData {
    url: String,
}

// JSON-structured response for continuations
#[derive(Debug, Deserialize)]
struct ContinuationsJsonResponse {
    continuations: Vec<ContinuationItem>,
}

#[derive(Debug, Deserialize)]
struct ContinuationItem {
    title: String,
    content: String,
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

    /// Generate image using OpenAI DALL-E
    #[instrument(skip(self, context, node))]
    pub async fn generate_image(
        &self,
        context: &ImageStoryContext,
        node: &NodeContext,
        params: &ImageParams,
        account_tier: &AccountTier,
    ) -> Result<(Vec<u8>, GeneratedImage)> {
        let openai_key = self
            .config
            .openai_api_key
            .as_ref()
            .ok_or_else(|| ApiError::AIProvider("OpenAI API key not configured".to_string()))?;

        // Build image prompt
        let prompt = self.build_image_prompt(context, node, params);

        // Determine size and quality based on resolution and tier
        let (size, quality) = match params.resolution.as_str() {
            "high" | "hd" => {
                // Pro tier: HD quality with larger size
                match params.aspect_ratio.as_str() {
                    "3:4" => ("1536x2048", "hd"),
                    _ => ("1024x1024", "hd"),
                }
            }
            _ => {
                // Free tier or medium: standard quality
                match params.aspect_ratio.as_str() {
                    "3:4" => ("768x1024", "standard"),
                    _ => ("1024x1024", "standard"),
                }
            }
        };

        let request = OpenAIImageRequest {
            model: "dall-e-3".to_string(),
            prompt: prompt.clone(),
            n: 1,
            size: size.to_string(),
            quality: quality.to_string(),
        };

        info!(
            "Generating image with DALL-E 3: size={}, quality={}, prompt_len={}",
            size,
            quality,
            prompt.len()
        );

        // Call OpenAI API
        let response = self
            .http_client
            .post("https://api.openai.com/v1/images/generations")
            .header("Authorization", format!("Bearer {}", openai_key))
            .json(&request)
            .timeout(std::time::Duration::from_secs(60))
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

        // Download the generated image
        info!("Downloading generated image from: {}", image_url);
        let image_response = self
            .http_client
            .get(&image_url)
            .timeout(std::time::Duration::from_secs(30))
            .send()
            .await
            .map_err(|e| ApiError::AIProvider(format!("Failed to download image: {}", e)))?;

        if !image_response.status().is_success() {
            return Err(ApiError::AIProvider(format!(
                "Image download failed with status: {}",
                image_response.status()
            )));
        }

        let image_bytes = image_response
            .bytes()
            .await
            .map_err(|e| ApiError::AIProvider(format!("Failed to read image bytes: {}", e)))?
            .to_vec();

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
            "Generated and downloaded image: {}x{}, {} bytes",
            width,
            height,
            image_bytes.len()
        );

        let generated_image = GeneratedImage {
            url: String::new(), // Will be replaced with storage URL by caller
            mime_type: "image/png".to_string(),
            width,
            height,
        };

        Ok((image_bytes, generated_image))
    }

    // ==================== Prompt Section Formatters ====================

    /// Format background section for prompts
    fn format_background_section(bg: &Option<Background>) -> String {
        bg.as_ref()
            .and_then(|b| {
                let mut lines = Vec::new();

                if let Some(genre) = &b.genre {
                    if !genre.is_empty() {
                        lines.push(format!("- Genre: {}", genre));
                    }
                }

                if let Some(tone) = &b.tone {
                    if !tone.is_empty() {
                        lines.push(format!("- Tone: {}", tone));
                    }
                }

                if let Some(setting) = &b.setting {
                    if !setting.is_empty() {
                        lines.push(format!("- Setting: {}", setting));
                    }
                }

                if lines.is_empty() {
                    None
                } else {
                    Some(format!("\nBackground:\n{}\n", lines.join("\n")))
                }
            })
            .unwrap_or_default()
    }

    /// Format characters section for prompts (full detail)
    fn format_characters_section(chars: &Option<Vec<Character>>) -> String {
        chars
            .as_ref()
            .and_then(|characters| {
                if characters.is_empty() {
                    return None;
                }

                let mut lines = Vec::new();
                for character in characters {
                    let mut char_line = format!("- {}", character.name);

                    if let Some(role) = &character.role {
                        if !role.is_empty() {
                            char_line.push_str(&format!(" ({})", role));
                        }
                    }

                    lines.push(char_line);

                    if let Some(desc) = &character.description {
                        if !desc.is_empty() {
                            lines.push(format!("  {}", desc));
                        }
                    }
                }

                Some(format!("\nCharacters:\n{}\n", lines.join("\n")))
            })
            .unwrap_or_default()
    }

    /// Format characters section for ideas prompts (compact)
    fn format_characters_compact(chars: &Option<Vec<Character>>) -> String {
        chars
            .as_ref()
            .and_then(|characters| {
                if characters.is_empty() {
                    return None;
                }

                let names: Vec<String> = characters
                    .iter()
                    .map(|c| {
                        if let Some(role) = &c.role {
                            if !role.is_empty() {
                                return format!("{} ({})", c.name, role);
                            }
                        }
                        c.name.clone()
                    })
                    .collect();

                Some(format!("Characters: {}\n", names.join(", ")))
            })
            .unwrap_or_default()
    }

    /// Format background section for ideas prompts (compact)
    fn format_background_compact(bg: &Option<Background>) -> String {
        bg.as_ref()
            .and_then(|b| {
                let parts: Vec<String> = vec![
                    b.genre.as_ref().filter(|s| !s.is_empty()).cloned(),
                    b.tone.as_ref().filter(|s| !s.is_empty()).cloned(),
                    b.setting.as_ref().filter(|s| !s.is_empty()).cloned(),
                ]
                .into_iter()
                .flatten()
                .collect();

                if parts.is_empty() {
                    None
                } else {
                    Some(format!("Background: {}\n", parts.join(", ")))
                }
            })
            .unwrap_or_default()
    }

    /// Format story content from nodes (full detail for prose)
    fn format_story_content_detailed(nodes: &[PathNode]) -> String {
        if nodes.len() > 5 {
            let split_at = nodes.len() - 3;
            let (earlier, recent) = nodes.split_at(split_at);

            let mut content = String::new();

            if !earlier.is_empty() {
                content.push_str("Story so far:\n");
                for (i, node) in earlier.iter().enumerate() {
                    if let Some(summary) = &node.summary {
                        if !summary.is_empty() {
                            content.push_str(&format!("{}. {}\n", i + 1, summary));
                        }
                    } else if !node.content.is_empty() {
                        let preview: String = node.content.chars().take(100).collect();
                        content.push_str(&format!("{}. {}...\n", i + 1, preview));
                    }
                }
                content.push_str("\n");
            }

            content.push_str("Recent events:\n");
            for node in recent {
                if !node.content.is_empty() {
                    content.push_str(&format!("{}\n\n", node.content));
                }
            }

            content
        } else {
            let mut content = String::from("Story so far:\n");
            for node in nodes {
                if !node.content.is_empty() {
                    content.push_str(&format!("{}\n\n", node.content));
                }
            }
            content
        }
    }

    /// Format story content from nodes (summarized for ideas)
    fn format_story_content_summary(nodes: &[PathNode]) -> String {
        let mut content = String::new();

        if nodes.len() > 3 {
            let split_at = nodes.len() - 2;
            let (earlier, recent) = nodes.split_at(split_at);

            if !earlier.is_empty() {
                content.push_str("Story context:\n");
                for node in earlier {
                    if let Some(summary) = &node.summary {
                        if !summary.is_empty() {
                            content.push_str(&format!("- {}\n", summary));
                        }
                    } else if !node.content.is_empty() {
                        let preview: String = node.content.chars().take(80).collect();
                        content.push_str(&format!("- {}...\n", preview));
                    }
                }
                content.push_str("\n");
            }

            content.push_str("Current situation:\n");
            for node in recent {
                if let Some(summary) = &node.summary {
                    if !summary.is_empty() {
                        content.push_str(&format!("- {}\n", summary));
                    }
                } else if !node.content.is_empty() {
                    let preview: String = node.content.chars().take(150).collect();
                    content.push_str(&format!("- {}\n", preview));
                }
            }
        } else {
            content.push_str("Story so far:\n");
            for node in nodes {
                if let Some(summary) = &node.summary {
                    if !summary.is_empty() {
                        content.push_str(&format!("- {}\n", summary));
                    }
                } else if !node.content.is_empty() {
                    content.push_str(&format!("- {}\n", node.content));
                }
            }
        }

        content
    }

    /// Format generation instructions for prose
    fn format_prose_instructions(
        params: &GenerationParams,
        instructions: Option<&str>,
        has_context: bool,
    ) -> String {
        let mut inst = format!(
            "Generate {} different prose continuations ({}-{} words each).\n",
            params.num_candidates, params.min_words, params.max_words
        );

        if has_context {
            inst.push_str("Consider the established characters, tone, and setting.\n");
        }

        if let Some(tone) = &params.tone {
            inst.push_str(&format!("Tone: {}\n", tone));
        }

        if params.avoid_hard_end {
            inst.push_str("Avoid definitive endings; leave room for further branches.\n");
        }

        if let Some(user_inst) = instructions {
            inst.push_str(&format!("\nAdditional instructions: {}\n", user_inst));
        }

        inst
    }

    /// Format focus areas for ideas generation
    fn format_ideas_focus(chars: &Option<Vec<Character>>) -> String {
        if let Some(characters) = chars {
            if !characters.is_empty() {
                let names: Vec<&str> = characters.iter().map(|c| c.name.as_str()).collect();
                return format!(
                    "{} decisions, plot directions, or setting changes",
                    names.join("/")
                );
            }
        }
        "character decisions, plot directions, or setting changes".to_string()
    }

    /// Format generation instructions for ideas
    fn format_ideas_instructions(
        params: &GenerationParams,
        instructions: Option<&str>,
        focus_areas: &str,
    ) -> String {
        let mut inst = format!(
            "Generate {} different continuation ideas ({}-{} words each). Each should suggest a distinct branch the story could take.\nFocus on: {}.",
            params.num_candidates,
            params.min_words,
            params.max_words,
            focus_areas
        );

        if let Some(tone) = &params.tone {
            inst.push_str(&format!("\nTone: {}", tone));
        }

        if params.avoid_hard_end {
            inst.push_str("\nAvoid definitive endings; leave room for further branching.");
        }

        if let Some(user_inst) = instructions {
            inst.push_str(&format!("\n\nAdditional instructions: {}", user_inst));
        }

        inst
    }

    // ==================== End Prompt Section Formatters ====================

    /// Build prompt for text generation (template-based)
    fn build_text_prompt(
        &self,
        context: &StoryContext,
        nodes: &[PathNode],
        params: &GenerationParams,
        instructions: Option<&str>,
    ) -> String {
        let title = context.title.as_deref().unwrap_or("Untitled");
        let language = &context.language;

        let tags = if !context.tags.is_empty() {
            format!("Genre/Tags: {}\n", context.tags.join(", "))
        } else {
            String::new()
        };

        let background = Self::format_background_section(&context.background);
        let characters = Self::format_characters_section(&context.active_characters);
        let story_content = Self::format_story_content_detailed(nodes);

        let has_context =
            context.background.is_some() || context.active_characters.is_some();
        let generation_instructions =
            Self::format_prose_instructions(params, instructions, has_context);

        format!(
            r#"Story: {title}
Language: {language}
{tags}{background}{characters}
{story_content}
{generation_instructions}"#,
            title = title,
            language = language,
            tags = tags,
            background = background,
            characters = characters,
            story_content = story_content,
            generation_instructions = generation_instructions,
        )
    }

    /// Build prompt for image generation
    fn build_image_prompt(
        &self,
        context: &ImageStoryContext,
        node: &NodeContext,
        params: &ImageParams,
    ) -> String {
        let mut prompt = String::new();

        // Style description
        let style = params.style.unwrap_or(ImageStyle::Illustration);
        prompt.push_str(&format!("{} style illustration: ", style.as_str()));

        // Content (prefer content over summary)
        if let Some(content) = &node.content {
            prompt.push_str(content);
        } else if let Some(summary) = &node.summary {
            prompt.push_str(summary);
        }

        // Tags
        if !node.tags.is_empty() {
            prompt.push_str(&format!(", featuring: {}", node.tags.join(", ")));
        }

        prompt
    }

    /// Build prompt for ideas generation (template-based)
    fn build_ideas_prompt(
        &self,
        context: &StoryContext,
        nodes: &[PathNode],
        params: &GenerationParams,
        instructions: Option<&str>,
    ) -> String {
        let title = context.title.as_deref().unwrap_or("Untitled");
        let language = &context.language;

        let tags = if !context.tags.is_empty() {
            format!("Genre: {}\n", context.tags.join(", "))
        } else {
            String::new()
        };

        let background = Self::format_background_compact(&context.background);
        let characters = Self::format_characters_compact(&context.active_characters);
        let story_content = Self::format_story_content_summary(nodes);
        let focus_areas = Self::format_ideas_focus(&context.active_characters);
        let generation_instructions =
            Self::format_ideas_instructions(params, instructions, &focus_areas);

        format!(
            r#"Story: {title}
Language: {language}
{tags}{background}{characters}
{story_content}
{generation_instructions}"#,
            title = title,
            language = language,
            tags = tags,
            background = background,
            characters = characters,
            story_content = story_content,
            generation_instructions = generation_instructions,
        )
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
            response_format: None,
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
    #[instrument(skip(self, story_context, nodes, account_tier))]
    pub async fn generate_summaries(
        &self,
        story_context: Option<&StoryContextSimple>,
        nodes: &[NodeToSummarize],
        account_tier: &AccountTier,
    ) -> Result<Vec<NodeSummary>> {
        // Build system prompt
        let system_prompt =
            "You are a writing assistant that generates concise one-line summaries. \
            Each summary should be max 50 characters and capture the key action or event.";

        // Build user prompt with story context
        let mut user_prompt = String::new();

        // Add story context if provided
        if let Some(context) = story_context {
            if let Some(title) = &context.title {
                if !title.is_empty() {
                    user_prompt.push_str(&format!("Story: {}\n", title));
                }
            }

            if let Some(language) = &context.language {
                if !language.is_empty() {
                    user_prompt.push_str(&format!("Language: {}\n", language));
                }
            }

            if !context.tags.is_empty() {
                user_prompt.push_str(&format!("Genre: {}\n", context.tags.join(", ")));
            }

            if user_prompt.len() > 0 {
                user_prompt.push_str("\n");
            }
        }

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
            response_format: None,
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

    /// Generate prose story continuations using JSON-structured output
    #[instrument(skip(self, context, nodes, account_tier))]
    pub async fn generate_prose_continuations(
        &self,
        context: &StoryContext,
        nodes: &[PathNode],
        params: &GenerationParams,
        instructions: Option<&str>,
        account_tier: &AccountTier,
    ) -> Result<Vec<TextCandidate>> {
        // Build JSON-requesting prompt
        let system_prompt = r#"You are a creative writing assistant for Talevonia, a branching narrative app.
Return your response as a JSON object with this exact structure:
{
  "continuations": [
    {
      "title": "Brief 4-8 word summary",
      "content": "Full prose continuation (follow word count requirements)"
    }
  ]
}

Each continuation should present a different plot direction, character choice, or narrative possibility."#.to_string();

        let mut user_prompt = self.build_text_prompt(context, nodes, params, instructions);
        user_prompt.push_str("\n\nReturn the response in JSON format as specified.");

        // Choose model
        let input_chars = user_prompt.len() + system_prompt.len();
        let model = self.select_model(TaskKind::Continue, account_tier, input_chars)?;

        // Prepare request with JSON format
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
            max_tokens: (params.max_words * params.num_candidates as u32 * 2) as u32,
            temperature: 0.7,
            n: 1,  // Single response with JSON array
            response_format: Some(ResponseFormat {
                format_type: "json_object".to_string(),
            }),
        };

        // Call API with retry logic
        let response_text = self.call_openrouter_api(request).await?;

        // Parse JSON response
        let json_response: ContinuationsJsonResponse = serde_json::from_str(&response_text)
            .map_err(|e| {
                ApiError::AIProvider(format!("Failed to parse JSON response: {}. Response: {}", e, response_text))
            })?;

        // Convert to TextCandidate
        let candidates: Vec<TextCandidate> = json_response
            .continuations
            .into_iter()
            .map(|item| TextCandidate {
                id: Uuid::new_v4().to_string(),
                content: item.content,
                title: Some(item.title),
                safety_flags: vec![],
            })
            .collect();

        info!(
            "Generated {} prose continuations using model {} (JSON format)",
            candidates.len(),
            model.model
        );

        Ok(candidates)
    }

    /// Generate high-level continuation ideas using JSON-structured output
    #[instrument(skip(self, context, nodes, account_tier))]
    pub async fn generate_continuation_ideas(
        &self,
        context: &StoryContext,
        nodes: &[PathNode],
        params: &GenerationParams,
        instructions: Option<&str>,
        account_tier: &AccountTier,
    ) -> Result<Vec<TextCandidate>> {
        // Build JSON-requesting prompt with dynamic word counts
        let system_prompt = format!(
            r#"You are a creative writing assistant for Talevonia, a branching narrative app.
Return your response as a JSON object with this exact structure:
{{
  "continuations": [
    {{
      "title": "4-12 word summary of this branch direction",
      "content": "{}-{} word description of what happens in this branch"
    }}
  ]
}}

Each idea should suggest a distinct narrative direction: character decision, plot twist, or setting change."#,
            params.min_words, params.max_words
        );

        let mut user_prompt = self.build_ideas_prompt(context, nodes, params, instructions);
        user_prompt.push_str("\n\nReturn the response in JSON format as specified.");

        // Choose model
        let input_chars = user_prompt.len() + system_prompt.len();
        let model = self.select_model(TaskKind::Ideas, account_tier, input_chars)?;

        // Prepare request with JSON format - calculate max_tokens based on params
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
            max_tokens: (params.max_words * params.num_candidates as u32 * 2) as u32,  // 2x for titles + JSON overhead
            temperature: 0.8,  // Higher creativity for ideas
            n: 1,  // Single response with JSON array
            response_format: Some(ResponseFormat {
                format_type: "json_object".to_string(),
            }),
        };

        // Call API with retry logic
        let response_text = self.call_openrouter_api(request).await?;

        // Parse JSON response
        let json_response: ContinuationsJsonResponse = serde_json::from_str(&response_text)
            .map_err(|e| {
                ApiError::AIProvider(format!("Failed to parse JSON response: {}. Response: {}", e, response_text))
            })?;

        // Convert to TextCandidate
        let candidates: Vec<TextCandidate> = json_response
            .continuations
            .into_iter()
            .map(|item| TextCandidate {
                id: Uuid::new_v4().to_string(),
                content: item.content,
                title: Some(item.title),
                safety_flags: vec![],
            })
            .collect();

        info!(
            "Generated {} continuation ideas using model {} (JSON format)",
            candidates.len(),
            model.model
        );

        Ok(candidates)
    }

    /// Helper function to call OpenRouter API with retry logic
    async fn call_openrouter_api(&self, request: OpenAIRequest) -> Result<String> {
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
                            last_err = Some(format!("OpenRouter error {}: {}", status.as_u16(), text));
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

                    // Extract content from first choice
                    if let Some(first_choice) = openai_response.choices.first() {
                        return Ok(first_choice.message.content.clone());
                    } else {
                        return Err(ApiError::AIProvider("No choices in response".to_string()));
                    }
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
}
