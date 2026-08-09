use bevy::asset::RenderAssetUsages;
use bevy::image::{Image, ImageSampler};
use bevy::prelude::*;
use bevy::render::render_resource::{Extent3d, TextureDimension, TextureFormat};

/// The two round textures the viewer draws on the turf, generated rather than
/// shipped: a contact shadow and the team ring under a player's boots.
///
/// Both are white with a shaped alpha channel, so the material's base colour
/// decides what they end up looking like and one image serves every player.
pub struct Textures;

impl Textures {
    const SIZE: u32 = 96;

    /// A soft round shadow. Real shadow maps are the single most expensive
    /// thing this scene could ask a WebGL2 context for, and twenty-two figures
    /// standing on nothing look like they are hovering.
    pub fn blob(images: &mut Assets<Image>) -> Handle<Image> {
        images.add(Self::radial(|distance| {
            // Dense under the player, gone by the edge of the quad.
            Self::smooth(((1.0 - distance) / 0.75).clamp(0.0, 1.0))
        }))
    }

    /// The ring drawn round a player's feet in their team's colour.
    pub fn ring(images: &mut Assets<Image>) -> Handle<Image> {
        images.add(Self::radial(|distance| {
            const RADIUS: f32 = 0.80;
            const WIDTH: f32 = 0.17;
            Self::smooth(1.0 - ((distance - RADIUS) / WIDTH).abs().min(1.0))
        }))
    }

    /// White throughout, with `alpha` sampled on the distance from the centre —
    /// 0 at the middle of the image, 1 at the edge of the inscribed circle.
    fn radial(alpha: impl Fn(f32) -> f32) -> Image {
        let mut data = Vec::with_capacity((Self::SIZE * Self::SIZE * 4) as usize);
        let centre = (Self::SIZE as f32 - 1.0) * 0.5;
        for row in 0..Self::SIZE {
            for column in 0..Self::SIZE {
                let offset = Vec2::new(column as f32 - centre, row as f32 - centre) / centre;
                let value = (alpha(offset.length()).clamp(0.0, 1.0) * 255.0) as u8;
                data.extend_from_slice(&[255, 255, 255, value]);
            }
        }

        let mut image = Image::new(
            Extent3d {
                width: Self::SIZE,
                height: Self::SIZE,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba8UnormSrgb,
            RenderAssetUsages::RENDER_WORLD,
        );
        // Both textures are gradients a couple of metres across on screen —
        // point sampling would draw them as a staircase.
        image.sampler = ImageSampler::linear();
        image
    }

    fn smooth(value: f32) -> f32 {
        value * value * (3.0 - 2.0 * value)
    }
}
