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

/// AI Text Continue Request
#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct AITextContinueRequest {
    #[serde(default)]
    pub mode: AITextContinueMode,
    #[validate(nested)]
    pub story_context: StoryContext,
    #[validate(length(min = 1, max = 50), nested)]
    pub path_nodes: Vec<PathNode>,
    #[serde(default)]
    #[validate(nested)]
    pub generation_params: GenerationParams,
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
}

#[derive(Debug, Deserialize, Serialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct PathNode {
    #[validate(length(max = 200))]
    pub summary: Option<String>,
    #[validate(length(min = 1, max = 50000))]
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
    pub success: bool,
    pub data: AITextContinueData,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AITextContinueData {
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

#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct ImageStoryContext {
    #[validate(length(max = 500))]
    pub title: Option<String>,
    #[serde(default = "default_language")]
    #[validate(length(min = 2, max = 10))]
    pub language: String,
}

#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct NodeContext {
    #[validate(length(max = 200))]
    pub summary: Option<String>,
    #[validate(length(min = 1, max = 10000))]
    pub content: String,
    #[serde(default)]
    #[validate(length(max = 20))]
    pub tags: Vec<String>,
}

#[derive(Debug, Deserialize, Validate)]
#[serde(rename_all = "camelCase")]
pub struct ImageParams {
    #[validate(length(max = 100))]
    pub style: Option<String>,
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
            style: None,
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
    pub success: bool,
    pub data: AIImageGenerateData,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AIImageGenerateData {
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
    pub success: bool,
    pub data: AITextEditData,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AITextEditData {
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

#[cfg(test)]
mod tests {
    use super::*;
    use validator::Validate;

    fn base_request() -> AITextContinueRequest {
        AITextContinueRequest {
            mode: AITextContinueMode::default(),
            story_context: StoryContext {
                title: Some("Test".to_string()),
                tags: vec!["tag".to_string()],
                language: "en".to_string(),
            },
            path_nodes: vec![PathNode {
                summary: Some("Summary".to_string()),
                content: "Valid content".to_string(),
            }],
            generation_params: GenerationParams::default(),
        }
    }

    #[test]
    fn rejects_invalid_path_node_content() {
        let mut request = base_request();
        request.path_nodes[0].content = "".to_string();

        assert!(
            request.validate().is_err(),
            "Empty path node content should fail validation"
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
