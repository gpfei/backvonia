use crate::{
    config::AIConfig,
    error::{ApiError, Result},
    models::ai::{
        AITextContinueMode, AITextEditMode, EditInput, EditParams, GeneratedImage,
        GenerationParams, ImageParams, ImageStoryContext, NodeContext, PathNode,
        StoryContext, StoryContextSimple, TextCandidate, TextEditCandidate,
    },
};
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
            .timeout(std::time::Duration::from_secs(60)) // 60s total timeout
            .connect_timeout(std::time::Duration::from_secs(10)) // 10s connection timeout
            .build()
            .expect("Failed to build HTTP client");

        Self {
            config: config.clone(),
            http_client,
        }
    }

    /// Generate text continuations using OpenAI
    #[instrument(skip(self, context, nodes))]
    pub async fn generate_text_continuations(
        &self,
        mode: AITextContinueMode,
        context: &StoryContext,
        nodes: &[PathNode],
        params: &GenerationParams,
    ) -> Result<Vec<TextCandidate>> {
        // Build the prompt based on mode
        let (system_prompt, user_prompt) = match mode {
            AITextContinueMode::Prose => {
                let system = format!(
                    "You are a creative writing assistant. Generate {} distinct story continuations in full prose.",
                    params.num_candidates
                );
                let user = self.build_text_prompt(context, nodes, params);
                (system, user)
            }
            AITextContinueMode::Ideas => {
                let system = format!(
                    "You are a creative writing assistant. Generate {} distinct high-level story continuation ideas or branch directions. For each idea, provide a brief title (3-8 words) and a short description (20-50 words).",
                    params.num_candidates
                );
                let user = self.build_ideas_prompt(context, nodes, params);
                (system, user)
            }
        };

        // Prepare OpenAI request
        let request = OpenAIRequest {
            model: "gpt-4".to_string(),
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
            temperature: 0.8,
            n: params.num_candidates,
        };

        // Call OpenAI API
        let response = self
            .http_client
            .post("https://api.openai.com/v1/chat/completions")
            .header(
                "Authorization",
                format!("Bearer {}", self.config.openai_api_key),
            )
            .json(&request)
            .send()
            .await
            .map_err(|e| ApiError::AIProvider(format!("OpenAI request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(ApiError::AIProvider(format!(
                "OpenAI API error: {}",
                error_text
            )));
        }

        let openai_response: OpenAIResponse = response
            .json()
            .await
            .map_err(|e| ApiError::AIProvider(format!("Failed to parse response: {}", e)))?;

        // Convert to candidates
        let candidates = openai_response
            .choices
            .into_iter()
            .map(|choice| {
                let content = choice.message.content;
                let (title, parsed_content) = if mode == AITextContinueMode::Ideas {
                    // Try to parse title from response
                    self.parse_idea_response(&content)
                } else {
                    (None, content)
                };

                TextCandidate {
                    id: Uuid::new_v4().to_string(),
                    content: parsed_content,
                    title,
                    safety_flags: vec![],
                }
            })
            .collect();

        info!("Generated {} text continuations in mode {:?}", params.num_candidates, mode);

        Ok(candidates)
    }

    /// Generate image using OpenAI DALL-E
    #[instrument(skip(self, context, node))]
    pub async fn generate_image(
        &self,
        context: &ImageStoryContext,
        node: &NodeContext,
        params: &ImageParams,
    ) -> Result<GeneratedImage> {
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
            prompt,
            n: 1,
            size: size.to_string(),
        };

        // Call OpenAI API
        let response = self
            .http_client
            .post("https://api.openai.com/v1/images/generations")
            .header(
                "Authorization",
                format!("Bearer {}", self.config.openai_api_key),
            )
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

        info!("Generated image with DALL-E 3");

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

        if let Some(title) = &context.title {
            prompt.push_str(&format!("Story: {}\n\n", title));
        }

        if !context.tags.is_empty() {
            prompt.push_str(&format!("Tags: {}\n\n", context.tags.join(", ")));
        }

        prompt.push_str("Previous story path:\n");
        for (i, node) in nodes.iter().enumerate() {
            prompt.push_str(&format!("{}. {}\n", i + 1, node.content));
        }

        prompt.push_str(&format!(
            "\n\nGenerate a continuation of {}-{} words. ",
            params.min_words, params.max_words
        ));

        if let Some(tone) = &params.tone {
            prompt.push_str(&format!("Tone: {}. ", tone));
        }

        if params.avoid_hard_end {
            prompt.push_str("Keep the story open-ended for further continuation. ");
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

        if let Some(title) = &context.title {
            prompt.push_str(&format!("Story: {}\n\n", title));
        }

        if !context.tags.is_empty() {
            prompt.push_str(&format!("Tags: {}\n\n", context.tags.join(", ")));
        }

        prompt.push_str("Previous story path:\n");
        for (i, node) in nodes.iter().enumerate() {
            prompt.push_str(&format!("{}. {}\n", i + 1, node.content));
        }

        prompt.push_str("\n\nFor each continuation idea, format as:\nTitle: [brief title]\n[description]\n\n");

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

        // Fallback: no title found
        (None, response.to_string())
    }

    /// Generate text edit/transformation using OpenAI
    #[instrument(skip(self, input, params))]
    pub async fn generate_text_edit(
        &self,
        mode: AITextEditMode,
        story_context: Option<&StoryContextSimple>,
        input: &EditInput,
        params: &EditParams,
    ) -> Result<Vec<TextEditCandidate>> {
        let system_prompt = self.build_edit_system_prompt(mode);
        let user_prompt = self.build_edit_user_prompt(mode, story_context, input, params);

        // Prepare OpenAI request with configurable number of candidates
        let request = OpenAIRequest {
            model: "gpt-4".to_string(),
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

        // Call OpenAI API
        let response = self
            .http_client
            .post("https://api.openai.com/v1/chat/completions")
            .header(
                "Authorization",
                format!("Bearer {}", self.config.openai_api_key),
            )
            .json(&request)
            .send()
            .await
            .map_err(|e| ApiError::AIProvider(format!("OpenAI edit request failed: {}", e)))?;

        if !response.status().is_success() {
            let error_text = response.text().await.unwrap_or_default();
            return Err(ApiError::AIProvider(format!(
                "OpenAI edit API error: {}",
                error_text
            )));
        }

        let openai_response: OpenAIResponse = response
            .json()
            .await
            .map_err(|e| ApiError::AIProvider(format!("Failed to parse edit response: {}", e)))?;

        // Convert to candidates
        let candidates = openai_response
            .choices
            .into_iter()
            .map(|choice| TextEditCandidate {
                id: Uuid::new_v4().to_string(),
                content: choice.message.content,
                safety_flags: vec![],
            })
            .collect();

        info!("Generated {} edit candidates in mode {:?}", params.num_candidates, mode);

        Ok(candidates)
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
}
