use crate::{
    config::StorageConfig,
    error::{ApiError, Result},
};
use aws_config::Region;
use aws_credential_types::Credentials;
use aws_sdk_s3::{
    config::Builder as S3ConfigBuilder, presigning::PresigningConfig, primitives::ByteStream,
    Client as S3Client,
};
use std::time::Duration;
use tracing::{info, instrument, warn};
use uuid::Uuid;

pub struct StorageService {
    client: S3Client,
    bucket_name: String,
    endpoint_url: String,
    public_base_url: Option<String>,
    signed_url_expiration: Duration,
}

impl StorageService {
    /// Create a new StorageService instance
    pub async fn new(config: &StorageConfig) -> Result<Self> {
        // Create AWS credentials
        let credentials = Credentials::new(
            &config.access_key_id,
            &config.secret_access_key,
            None,
            None,
            "CloudflareR2",
        );

        // Build S3 config for Cloudflare R2
        let s3_config = S3ConfigBuilder::new()
            .region(Region::new(config.region.clone()))
            .endpoint_url(&config.endpoint_url)
            .credentials_provider(credentials)
            .force_path_style(true) // Required for R2
            .behavior_version_latest()
            .build();

        let client = S3Client::from_conf(s3_config);

        let signed_url_expiration = Duration::from_secs(config.signed_url_expiration_seconds);

        info!(
            "StorageService initialized with bucket: {}, region: {}",
            config.bucket_name, config.region
        );

        Ok(Self {
            client,
            bucket_name: config.bucket_name.clone(),
            endpoint_url: config.endpoint_url.clone(),
            public_base_url: config.public_base_url.clone(),
            signed_url_expiration,
        })
    }

    /// Upload image bytes to storage and return the permanent URL
    #[instrument(skip(self, image_data))]
    pub async fn upload_image(
        &self,
        image_data: Vec<u8>,
        content_type: &str,
        user_id: Uuid,
    ) -> Result<(String, usize)> {
        // Generate unique filename: ai-images/{user_id}/{uuid}.jpg
        let file_id = Uuid::now_v7();
        let extension = match content_type {
            "image/png" => "png",
            "image/jpeg" | "image/jpg" => "jpg",
            "image/webp" => "webp",
            _ => "jpg", // Default to jpg
        };
        let key = format!("ai-images/{}/{}.{}", user_id, file_id, extension);

        let file_size = image_data.len();

        info!("Uploading image to R2: {} ({} bytes)", key, file_size);

        // Create ByteStream from data
        let byte_stream = ByteStream::from(image_data);

        // Upload to R2
        self.client
            .put_object()
            .bucket(&self.bucket_name)
            .key(&key)
            .body(byte_stream)
            .content_type(content_type)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to upload image to R2: {}", e);
                ApiError::Internal(anyhow::anyhow!("Failed to upload image: {}", e))
            })?;

        // Return permanent URL (either public base URL or R2 URL)
        let permanent_url = if let Some(base_url) = &self.public_base_url {
            format!("{}/{}", base_url, key)
        } else {
            // For R2, construct URL from endpoint and bucket
            format!("{}/{}/{}", self.endpoint_url, self.bucket_name, key)
        };

        info!("Image uploaded successfully: {}", permanent_url);

        Ok((permanent_url, file_size))
    }

    /// Generate a temporary signed URL for accessing the image
    #[instrument(skip(self))]
    pub async fn generate_signed_url(&self, key: &str) -> Result<String> {
        info!("Generating signed URL for key: {}", key);

        let presigning_config =
            PresigningConfig::expires_in(self.signed_url_expiration).map_err(|e| {
                warn!("Failed to create presigning config: {}", e);
                ApiError::Internal(anyhow::anyhow!(
                    "Failed to create signed URL configuration: {}",
                    e
                ))
            })?;

        let presigned_request = self
            .client
            .get_object()
            .bucket(&self.bucket_name)
            .key(key)
            .presigned(presigning_config)
            .await
            .map_err(|e| {
                warn!("Failed to generate signed URL: {}", e);
                ApiError::Internal(anyhow::anyhow!("Failed to generate signed URL: {}", e))
            })?;

        let signed_url = presigned_request.uri().to_string();

        info!(
            "Signed URL generated successfully (expires in {}s)",
            self.signed_url_expiration.as_secs()
        );

        Ok(signed_url)
    }

    /// Extract the key from a permanent URL
    pub fn extract_key_from_url(&self, url: &str) -> Option<String> {
        // Handle both public base URL and R2 URL formats
        if let Some(base_url) = &self.public_base_url {
            // Public URL format: https://cdn.example.com/ai-images/{user_id}/{file_id}.jpg
            url.strip_prefix(&format!("{}/", base_url))
                .map(|s| s.to_string())
        } else {
            // R2 URL format: https://account.r2.cloudflarestorage.com/bucket/ai-images/{user_id}/{file_id}.jpg
            url.split(&format!("{}/", self.bucket_name))
                .nth(1)
                .map(|s| s.to_string())
        }
    }

    /// Delete an image from storage
    #[instrument(skip(self))]
    pub async fn delete_image(&self, key: &str) -> Result<()> {
        info!("Deleting image from R2: {}", key);

        self.client
            .delete_object()
            .bucket(&self.bucket_name)
            .key(key)
            .send()
            .await
            .map_err(|e| {
                warn!("Failed to delete image from R2: {}", e);
                ApiError::Internal(anyhow::anyhow!("Failed to delete image: {}", e))
            })?;

        info!("Image deleted successfully: {}", key);

        Ok(())
    }
}
