use std::path::Path;

use crate::errors::WisecrowError;

const MAX_IMAGE_WIDTH: u32 = 200;
const MAX_IMAGE_HEIGHT: u32 = 200;

/// Fetches an image for a vocabulary word from Unsplash.
///
/// # Errors
///
/// Returns an error if the API request fails or no API key is configured.
pub async fn fetch_image(
    client: &reqwest::Client,
    word: &str,
    api_key: &str,
) -> Result<Vec<u8>, WisecrowError> {
    let url = url::Url::parse_with_params(
        "https://api.unsplash.com/photos/random",
        &[("query", word), ("orientation", "squarish")],
    )?;

    let response = client
        .get(url)
        .header("Authorization", format!("Client-ID {api_key}"))
        .send()
        .await?
        .error_for_status()?;

    let json: serde_json::Value = response.json().await?;

    let image_url = json["urls"]["small"]
        .as_str()
        .ok_or_else(|| WisecrowError::MediaError("No image URL in Unsplash response".to_owned()))?;

    let image_bytes = client
        .get(image_url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;

    resize_image(&image_bytes)
}

/// Resizes image bytes to fit within the maximum dimensions while
/// preserving aspect ratio.
///
/// # Errors
///
/// Returns an error if the image cannot be decoded.
pub fn resize_image(data: &[u8]) -> Result<Vec<u8>, WisecrowError> {
    let img = image::load_from_memory(data)
        .map_err(|e| WisecrowError::MediaError(format!("Failed to decode image: {e}")))?;

    let resized = img.resize(
        MAX_IMAGE_WIDTH,
        MAX_IMAGE_HEIGHT,
        image::imageops::FilterType::Lanczos3,
    );

    let mut buf = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut buf);
    resized
        .write_to(&mut cursor, image::ImageFormat::Jpeg)
        .map_err(|e| WisecrowError::MediaError(format!("Failed to encode image: {e}")))?;

    Ok(buf)
}

/// Loads a cached image and returns a ratatui-image stateful protocol
/// for rendering in the TUI.
///
/// # Errors
///
/// Returns an error if the image file cannot be read, decoded, or the
/// terminal does not support image protocols.
pub fn load_image_for_display(
    path: &Path,
) -> Result<Box<dyn ratatui_image::protocol::StatefulProtocol>, WisecrowError> {
    let dyn_img = image::ImageReader::open(path)
        .map_err(|e| WisecrowError::MediaError(format!("Failed to open image: {e}")))?
        .decode()
        .map_err(|e| WisecrowError::MediaError(format!("Failed to decode image: {e}")))?;

    let mut picker = ratatui_image::picker::Picker::from_termios()
        .map_err(|e| WisecrowError::MediaError(format!("Terminal does not support images: {e}")))?;

    Ok(picker.new_resize_protocol(dyn_img))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resize_preserves_valid_image() {
        let img = image::DynamicImage::new_rgb8(400, 300);
        let mut buf = Vec::new();
        let mut cursor = std::io::Cursor::new(&mut buf);
        img.write_to(&mut cursor, image::ImageFormat::Png).unwrap();

        let resized = resize_image(&buf).unwrap();
        let decoded = image::load_from_memory(&resized).unwrap();
        assert!(decoded.width() <= MAX_IMAGE_WIDTH);
        assert!(decoded.height() <= MAX_IMAGE_HEIGHT);
    }

    #[test]
    fn resize_rejects_invalid_data() {
        let result = resize_image(b"not an image");
        assert!(result.is_err());
    }
}
