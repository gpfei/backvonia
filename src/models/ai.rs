use serde::{Deserialize, Serialize};
use validator::Validate;

/// AI Text Continue Mode
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum AITextContinueMode {
    Prose, // Generate full prose continuation (default)
    Ideas, // Generate high-level continuation ideas or branch directions
}

impl Default for AITextContinueMode {
    fn default() -> Self {
        Self::Prose
    }
}

/// AI Text Continue Request (prose)
#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct AITextContinueRequest {
    #[validate(length(max = 2000))]
    pub instructions: Option<String>,
    #[validate(nested)]
    pub story_context: StoryContext,
    #[validate(length(min = 1, max = 50), nested)]
    pub path_nodes: Vec<PathNode>,
    #[serde(default)]
    #[validate(nested)]
    pub generation_params: GenerationParams,
}

/// AI Text Ideas Request
#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct AITextIdeasRequest {
    #[validate(length(max = 2000))]
    pub instructions: Option<String>,
    #[validate(nested)]
    pub story_context: StoryContext,
    #[validate(length(min = 1, max = 50), nested)]
    pub path_nodes: Vec<PathNode>,
    #[serde(default)]
    #[validate(nested)]
    pub generation_params: GenerationParams,
}

/// Story background context (genre, tone, setting)
#[derive(Debug, Deserialize, Serialize, Validate, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Background {
    /// Genre/category (e.g., "Fantasy", "Sci-Fi", "Mystery")
    #[validate(length(max = 100))]
    pub genre: Option<String>,
    /// Narrative tone (e.g., "Dark", "Humorous", "Suspenseful")
    #[validate(length(max = 100))]
    pub tone: Option<String>,
    /// Setting description (e.g., "Medieval castle", "Space station")
    #[validate(length(max = 500))]
    pub setting: Option<String>,
}

/// Character in the story
#[derive(Debug, Deserialize, Serialize, Validate, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Character {
    /// Character name (required)
    #[validate(length(min = 1, max = 100))]
    pub name: String,
    /// Character role (e.g., "Protagonist", "Antagonist", "Mentor")
    #[validate(length(max = 100))]
    pub role: Option<String>,
    /// Character description (personality, appearance, background)
    #[validate(length(max = 500))]
    pub description: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct StoryContext {
    #[validate(length(max = 500))]
    pub title: Option<String>,
    #[serde(default)]
    #[validate(length(max = 20))]
    pub tags: Vec<String>,
    #[serde(default = "default_language")]
    #[validate(length(min = 2, max = 10))]
    pub language: String,
    /// Story background (genre, tone, setting)
    #[serde(default)]
    #[validate(nested)]
    pub background: Option<Background>,
    /// Active characters in this story (max 8)
    #[serde(default)]
    #[validate(length(max = 8), nested)]
    pub active_characters: Option<Vec<Character>>,
}

#[derive(Debug, Deserialize, Serialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct PathNode {
    #[validate(length(max = 200))]
    pub summary: Option<String>,
    #[validate(length(max = 50000))]
    pub content: String,
}

#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct GenerationParams {
    #[serde(default = "default_num_candidates")]
    #[validate(range(min = 1, max = 5))]
    pub num_candidates: u8,
    #[serde(default = "default_min_words")]
    #[validate(range(min = 10, max = 500))]
    pub min_words: u32,
    #[serde(default = "default_max_words")]
    #[validate(range(min = 10, max = 1000))]
    pub max_words: u32,
    #[validate(length(max = 100))]
    pub tone: Option<String>,
    #[serde(default = "default_avoid_hard_end")]
    pub avoid_hard_end: bool,
}

impl Default for GenerationParams {
    fn default() -> Self {
        Self {
            num_candidates: default_num_candidates(),
            min_words: default_min_words(),
            max_words: default_max_words(),
            tone: None,
            avoid_hard_end: default_avoid_hard_end(),
        }
    }
}

fn default_language() -> String {
    "en".to_string()
}

fn default_num_candidates() -> u8 {
    3
}

fn default_min_words() -> u32 {
    80
}

fn default_max_words() -> u32 {
    200
}

fn default_avoid_hard_end() -> bool {
    true
}

/// AI Text Continue Response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AITextContinueResponse {
    pub candidates: Vec<TextCandidate>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextCandidate {
    pub id: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default)]
    pub safety_flags: Vec<String>,
}

/// Image generation style
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ImageStyle {
    Storybook,
    Anime,
    DigitalArt,
    Realistic,
    Watercolor,
    InkDrawing,
    ClassicalIllustration,
    Illustration,
}

impl Default for ImageStyle {
    fn default() -> Self {
        Self::Illustration
    }
}

impl ImageStyle {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Storybook => "storybook",
            Self::Anime => "anime",
            Self::DigitalArt => "digital-art",
            Self::Realistic => "realistic",
            Self::Watercolor => "watercolor",
            Self::InkDrawing => "ink-drawing",
            Self::ClassicalIllustration => "classical-illustration",
            Self::Illustration => "illustration",
        }
    }

    pub fn description(&self) -> &'static str {
        match self {
            Self::Storybook => "Book illustration style with whimsical, painterly qualities",
            Self::Anime => "Anime/manga art style with expressive characters",
            Self::DigitalArt => "Modern digital art with vibrant colors and clean lines",
            Self::Realistic => "Photorealistic style with detailed textures",
            Self::Watercolor => "Soft watercolor painting with flowing colors",
            Self::InkDrawing => "Dramatic ink drawing with strong contrast",
            Self::ClassicalIllustration => "Classical art illustration with renaissance aesthetics",
            Self::Illustration => "General illustration style, versatile and balanced",
        }
    }
}

/// AI Image Generate Request
#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct AIImageGenerateRequest {
    #[validate(nested)]
    pub story_context: ImageStoryContext,
    #[validate(nested)]
    pub node: NodeContext,
    #[serde(default)]
    #[validate(nested)]
    pub image_params: ImageParams,
}

/// Story context for image generation
#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct ImageStoryContext {
    #[validate(length(max = 500))]
    pub title: String,
    #[serde(default = "default_language")]
    #[validate(length(min = 2, max = 10))]
    pub language: String,
    #[validate(length(max = 100))]
    pub genre: Option<String>,
    #[validate(length(max = 100))]
    pub tone: Option<String>,
    #[validate(length(max = 500))]
    pub setting: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct NodeContext {
    #[validate(length(max = 200))]
    pub summary: Option<String>,
    #[validate(length(max = 10000))]
    pub content: Option<String>,
    #[serde(default)]
    #[validate(length(max = 20))]
    pub tags: Vec<String>,
}

impl NodeContext {
    /// Validates that at least one of summary or content is non-empty
    pub fn validate_has_content(&self) -> Result<(), &'static str> {
        let has_summary = self.summary.as_ref().map_or(false, |s| !s.trim().is_empty());
        let has_content = self.content.as_ref().map_or(false, |c| !c.trim().is_empty());

        if !has_summary && !has_content {
            return Err("Node must have summary or content");
        }
        Ok(())
    }
}

#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct ImageParams {
    #[serde(default)]
    pub style: Option<ImageStyle>,
    #[serde(default = "default_aspect_ratio")]
    #[validate(length(min = 3, max = 10))]
    pub aspect_ratio: String,
    #[serde(default = "default_resolution")]
    #[validate(length(min = 3, max = 20))]
    pub resolution: String,
}

impl Default for ImageParams {
    fn default() -> Self {
        Self {
            style: Some(ImageStyle::default()),
            aspect_ratio: default_aspect_ratio(),
            resolution: default_resolution(),
        }
    }
}

fn default_aspect_ratio() -> String {
    "3:4".to_string()
}

fn default_resolution() -> String {
    "medium".to_string()
}

/// AI Image Generate Response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AIImageGenerateResponse {
    pub image: GeneratedImage,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct GeneratedImage {
    pub url: String,
    pub mime_type: String,
    pub width: u32,
    pub height: u32,
}

/// AI Text Edit Mode
#[derive(Debug, Deserialize, Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum AITextEditMode {
    Expand,     // Expand
    Shorten,    // Shorten
    Rewrite,    // Rewrite
    FixGrammar, // Fix Grammar
}

/// AI Text Edit Request
#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct AITextEditRequest {
    pub mode: AITextEditMode,
    #[serde(default)]
    #[validate(nested)]
    pub story_context: Option<StoryContextSimple>,
    #[validate(nested)]
    pub input: EditInput,
    #[serde(default)]
    #[validate(nested)]
    pub edit_params: EditParams,
}

#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct StoryContextSimple {
    #[validate(length(max = 500))]
    pub title: Option<String>,
    #[validate(length(min = 2, max = 10))]
    pub language: Option<String>,
    #[serde(default)]
    #[validate(length(max = 20))]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct EditInput {
    #[validate(length(min = 1, max = 100000))]
    pub text: String,
    #[validate(length(max = 100000))]
    pub selection: Option<String>,
}

#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct EditParams {
    #[serde(default = "default_edit_candidates")]
    #[validate(range(min = 1, max = 5))]
    pub num_candidates: u8,
    #[validate(length(max = 20))]
    pub target_length: Option<String>, // "shorter", "similar", "longer"
    #[validate(length(max = 100))]
    pub tone: Option<String>,
    #[validate(length(min = 2, max = 10))]
    pub language: Option<String>,
    pub keep_style: Option<bool>,
}

impl Default for EditParams {
    fn default() -> Self {
        Self {
            num_candidates: default_edit_candidates(),
            target_length: None,
            tone: None,
            language: None,
            keep_style: Some(true),
        }
    }
}

fn default_edit_candidates() -> u8 {
    3
}

/// AI Text Edit Response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AITextEditResponse {
    pub mode: AITextEditMode,
    pub candidates: Vec<TextEditCandidate>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TextEditCandidate {
    pub id: String,
    pub content: String,
    #[serde(default)]
    pub safety_flags: Vec<String>,
}

/// AI Text Summarize Request
#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct AITextSummarizeRequest {
    #[serde(default)]
    #[validate(nested)]
    pub story_context: Option<StoryContextSimple>,
    #[validate(length(min = 1), nested)]
    pub nodes: Vec<NodeToSummarize>,
}

#[derive(Debug, Deserialize, Serialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct NodeToSummarize {
    #[validate(length(max = 100))]
    pub node_id: String,
    #[validate(length(min = 1, max = 50000))]
    pub content: String,
}

/// AI Text Summarize Response
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AITextSummarizeResponse {
    pub summaries: Vec<NodeSummary>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NodeSummary {
    pub node_id: String,
    pub summary: String,
}

#[cfg(test)]
mod tests {
    use super::*;
    use validator::Validate;

    fn base_request() -> AITextContinueRequest {
        AITextContinueRequest {
            instructions: None,
            story_context: StoryContext {
                title: Some("Test".to_string()),
                tags: vec!["tag".to_string()],
                language: "en".to_string(),
                background: None,
                active_characters: None,
            },
            path_nodes: vec![PathNode {
                summary: Some("Summary".to_string()),
                content: "Valid content".to_string(),
            }],
            generation_params: GenerationParams::default(),
        }
    }

    #[test]
    fn accepts_empty_chapter_node_content() {
        let mut request = base_request();
        request.path_nodes[0].content = "".to_string();
        request.path_nodes[0].summary = Some("Chapter 1".to_string());

        assert!(
            request.validate().is_ok(),
            "Empty chapter node content should be valid when summary exists"
        );
    }

    #[test]
    fn accepts_valid_path_nodes() {
        let request = base_request();
        assert!(
            request.validate().is_ok(),
            "Valid request should pass validation"
        );
    }
}
