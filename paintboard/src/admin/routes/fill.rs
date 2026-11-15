use std::ops::Deref;

use axum::{
    Json,
    extract::State,
    http::HeaderMap,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use base64::Engine;
use image::imageops::{self, FilterType};
use image::{DynamicImage, GenericImageView, ImageFormat};
use serde::{Deserialize, Serialize};
use tokio::task::spawn_blocking;

use crate::admin::routes::{admin_error, admin_ok, ensure_admin};
use crate::constants::{BOARD_HEIGHT, BOARD_WIDTH};
use crate::http::HttpState;
use crate::types::{PixelIndex, Rgba};

#[derive(Deserialize)]
pub struct FillRequest {
    pub area: Area,
    pub source: FillSource,
    #[serde(default)]
    pub filters: FillFilters,
}

#[derive(Deserialize)]
pub struct Area {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Deserialize)]
#[serde(tag = "kind", rename_all = "lowercase")]
pub enum FillSource {
    Color { value: String },
    Blob { value: String, format: String },
    Url { value: String, format: String },
}

#[derive(Deserialize, Default, Clone)]
pub struct FillFilters {
    #[serde(default)]
    pub scale: Option<ScaleFilter>,
    #[serde(default)]
    pub grayscale: bool,
    #[serde(default)]
    pub crop: Option<CropFilter>,
}

#[derive(Deserialize, Clone)]
pub struct ScaleFilter {
    pub mode: ScaleMode,
    pub width: u32,
    pub height: u32,
}

#[derive(Deserialize, Clone)]
#[serde(rename_all = "lowercase")]
pub enum ScaleMode {
    Fill,
    Contain,
    Cover,
}

#[derive(Deserialize, Clone)]
pub struct CropFilter {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

#[derive(Serialize)]
pub struct FillResponse {
    pub written: u32,
    pub format: &'static str,
}

pub async fn handle_fill(
    State(state): State<HttpState>,
    headers: HeaderMap,
    Json(payload): Json<FillRequest>,
) -> impl IntoResponse {
    let cfg = state.config.current();
    if let Err(resp) = ensure_admin(&headers, &cfg) {
        return resp;
    }

    if let Err(resp) = validate_area(&payload.area) {
        return resp;
    }

    let max_blob = cfg.admin_max_blob_bytes as usize;
    let target_w = payload.area.width;
    let target_h = payload.area.height;

    let result: Result<Vec<Rgba>, Response> = match &payload.source {
        FillSource::Color { value } => match parse_color(value) {
            Ok(color) => Ok(vec![color; (target_w * target_h) as usize]),
            Err(_) => Err(admin_error(StatusCode::BAD_REQUEST, 400, "INVALID_COLOR")),
        },
        FillSource::Blob { value, format } => {
            let decoded = match base64::engine::general_purpose::STANDARD.decode(value) {
                Ok(bytes) => bytes,
                Err(_) => return admin_error(StatusCode::BAD_REQUEST, 400, "INVALID_BASE64"),
            };
            if decoded.len() > max_blob {
                return admin_error(StatusCode::PAYLOAD_TOO_LARGE, 413, "BLOB_TOO_LARGE");
            }
            let format = match parse_format(format) {
                Some(f) => f,
                None => {
                    return admin_error(
                        StatusCode::UNSUPPORTED_MEDIA_TYPE,
                        415,
                        "UNSUPPORTED_FORMAT",
                    );
                }
            };
            process_image(
                decoded,
                format,
                payload.filters.clone(),
                target_w,
                target_h,
                max_blob,
            )
            .await
        }
        FillSource::Url { value, format } => {
            if !value.starts_with("https://") {
                return admin_error(StatusCode::BAD_REQUEST, 400, "URL_NOT_ALLOWED");
            }
            let format = match parse_format(format) {
                Some(f) => f,
                None => {
                    return admin_error(
                        StatusCode::UNSUPPORTED_MEDIA_TYPE,
                        415,
                        "UNSUPPORTED_FORMAT",
                    );
                }
            };
            match fetch_remote_image(value, max_blob).await {
                Ok(bytes) => {
                    process_image(
                        bytes,
                        format,
                        payload.filters.clone(),
                        target_w,
                        target_h,
                        max_blob,
                    )
                    .await
                }
                Err(resp) => Err(resp),
            }
        }
    };

    let pixels = match result {
        Ok(p) => p,
        Err(resp) => return resp,
    };

    let updates = to_updates(&payload.area, &pixels);
    let written = updates.len() as u32;
    state.pipeline.apply_admin_fill(updates);

    admin_ok(FillResponse {
        written,
        format: "rgb24",
    })
}

fn validate_area(area: &Area) -> Result<(), Response> {
    if area.width == 0 || area.height == 0 {
        return Err(admin_error(StatusCode::BAD_REQUEST, 400, "EMPTY_AREA").into_response());
    }
    if area.x >= BOARD_WIDTH || area.y >= BOARD_HEIGHT {
        return Err(admin_error(StatusCode::BAD_REQUEST, 400, "OUT_OF_BOUNDS").into_response());
    }
    if area.x.saturating_add(area.width) > BOARD_WIDTH
        || area.y.saturating_add(area.height) > BOARD_HEIGHT
    {
        return Err(admin_error(StatusCode::BAD_REQUEST, 400, "OUT_OF_BOUNDS").into_response());
    }
    Ok(())
}

fn parse_format(input: &str) -> Option<ImageFormat> {
    match input.to_ascii_lowercase().deref() {
        "png" => Some(ImageFormat::Png),
        "jpg" | "jpeg" => Some(ImageFormat::Jpeg),
        "webp" => Some(ImageFormat::WebP),
        _ => None,
    }
}

fn parse_color(input: &str) -> Result<Rgba, ()> {
    let trimmed = input.trim();
    let hex = trimmed.strip_prefix('#').unwrap_or(trimmed);
    if hex.len() != 6 {
        return Err(());
    }
    let r = u8::from_str_radix(&hex[0..2], 16).map_err(|_| ())?;
    let g = u8::from_str_radix(&hex[2..4], 16).map_err(|_| ())?;
    let b = u8::from_str_radix(&hex[4..6], 16).map_err(|_| ())?;
    Ok(Rgba(((r as u32) << 16) | ((g as u32) << 8) | b as u32))
}

async fn fetch_remote_image(url: &str, max_size: usize) -> Result<Vec<u8>, Response> {
    let resp = match reqwest::get(url).await {
        Ok(resp) => resp,
        Err(_) => {
            return Err(admin_error(StatusCode::BAD_REQUEST, 400, "FETCH_FAILED"));
        }
    };
    if let Some(len) = resp.content_length() {
        if len as usize > max_size {
            return Err(admin_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                413,
                "REMOTE_TOO_LARGE",
            ));
        }
    }
    match resp.bytes().await {
        Ok(bytes) => {
            if bytes.len() > max_size {
                Err(admin_error(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    413,
                    "REMOTE_TOO_LARGE",
                ))
            } else {
                Ok(bytes.to_vec())
            }
        }
        Err(_) => Err(admin_error(StatusCode::BAD_REQUEST, 400, "FETCH_FAILED")),
    }
}

async fn process_image(
    bytes: Vec<u8>,
    format: ImageFormat,
    filters: FillFilters,
    target_w: u32,
    target_h: u32,
    max_blob: usize,
) -> Result<Vec<Rgba>, Response> {
    spawn_blocking(move || {
        let image = image::load_from_memory_with_format(&bytes, format)
            .map_err(|_| admin_error(StatusCode::BAD_REQUEST, 400, "DECODE_FAILED"))?;
        let processed = apply_filters(image, filters, target_w, target_h)?;
        let (w, h) = processed.dimensions();
        if (w, h) != (target_w, target_h) {
            return Err(admin_error(
                StatusCode::BAD_REQUEST,
                400,
                "DIMENSION_MISMATCH",
            ));
        }
        if processed.as_bytes().len() > max_blob * 4 {
            // Safety guard for pathological inputs
            return Err(admin_error(
                StatusCode::PAYLOAD_TOO_LARGE,
                413,
                "IMAGE_TOO_LARGE",
            ));
        }
        let mut out = Vec::with_capacity((target_w * target_h) as usize);
        let rgb = processed.to_rgb8();
        for chunk in rgb.chunks_exact(3) {
            let rgba =
                Rgba(((chunk[0] as u32) << 16) | ((chunk[1] as u32) << 8) | (chunk[2] as u32));
            out.push(rgba);
        }
        Ok::<_, Response>(out)
    })
    .await
    .unwrap_or_else(|_| Err(admin_error(StatusCode::BAD_REQUEST, 400, "DECODE_FAILED")))
}

fn apply_filters(
    mut image: DynamicImage,
    filters: FillFilters,
    target_w: u32,
    target_h: u32,
) -> Result<DynamicImage, Response> {
    let scale = filters.scale.unwrap_or(ScaleFilter {
        mode: ScaleMode::Fill,
        width: target_w,
        height: target_h,
    });

    if scale.width == 0 || scale.height == 0 {
        return Err(admin_error(StatusCode::BAD_REQUEST, 400, "INVALID_SCALE"));
    }

    image = match scale.mode {
        ScaleMode::Fill => image.resize_exact(scale.width, scale.height, FilterType::Triangle),
        ScaleMode::Contain => {
            let resized = imageops::thumbnail(&image, scale.width, scale.height);
            let resized = DynamicImage::ImageRgba8(resized).to_rgb8();
            let mut canvas = image::RgbImage::new(scale.width, scale.height);
            imageops::replace(&mut canvas, &resized, 0, 0);
            DynamicImage::ImageRgb8(canvas)
        }
        ScaleMode::Cover => {
            let (w, h) = image.dimensions();
            let scale_x = scale.width as f32 / w as f32;
            let scale_y = scale.height as f32 / h as f32;
            let factor = scale_x.max(scale_y);
            let new_w = (w as f32 * factor).round().max(1.0) as u32;
            let new_h = (h as f32 * factor).round().max(1.0) as u32;
            let resized = image.resize(new_w, new_h, FilterType::Triangle);
            let offset_x = ((new_w - scale.width) / 2).min(new_w - 1);
            let offset_y = ((new_h - scale.height) / 2).min(new_h - 1);
            resized.crop_imm(offset_x, offset_y, scale.width, scale.height)
        }
    };

    if let Some(crop) = filters.crop {
        if crop.width == 0 || crop.height == 0 {
            return Err(admin_error(StatusCode::BAD_REQUEST, 400, "INVALID_CROP"));
        }
        let (w, h) = image.dimensions();
        if crop.x.saturating_add(crop.width) > w || crop.y.saturating_add(crop.height) > h {
            return Err(admin_error(StatusCode::BAD_REQUEST, 400, "INVALID_CROP"));
        }
        image = image.crop_imm(crop.x, crop.y, crop.width, crop.height);
    }

    if filters.grayscale {
        image = image.grayscale();
    }

    Ok(image)
}

fn to_updates(area: &Area, pixels: &[Rgba]) -> Vec<(PixelIndex, Rgba)> {
    let mut updates = Vec::with_capacity(pixels.len());
    for (idx, rgba) in pixels.iter().enumerate() {
        let x = area.x + (idx as u32 % area.width);
        let y = area.y + (idx as u32 / area.width);
        if let Some(px) = PixelIndex::from_coords(x as u16, y as u16) {
            updates.push((px, *rgba));
        }
    }
    updates
}
