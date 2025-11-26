use crate::{
    config::AIConfig,
    error::{ApiError, Result},
    models::ai::{
        GeneratedImage, GenerationParams, ImageParams, ImageStoryContext, NodeContext, PathNode,
        StoryContext, TextCandidate,
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
        context: &StoryContext,
        nodes: &[PathNode],
        params: &GenerationParams,
    ) -> Result<Vec<TextCandidate>> {
        // Build the prompt
        let system_prompt = format!(
            "You are a creative writing assistant. Generate {} distinct story continuations.",
            params.num_candidates
        );

        let user_prompt = self.build_text_prompt(context, nodes, params);

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
            .map(|choice| TextCandidate {
                id: Uuid::new_v4().to_string(),
                content: choice.message.content,
                safety_flags: vec![],
            })
            .collect();

        info!("Generated {} text continuations", params.num_candidates);

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
}
