//! The twenty approved home surfaces; one mesh and one shared grass texture.
//! Uses the viewer's grass, wear, UVs, tangent generation and material rasteriser.
use super::*;

#[path = "styles.rs"]
mod styles;

#[derive(Debug)]
pub(super) struct Style {
    pub(super) id: u8,
    #[cfg(test)]
    pub(super) name: &'static str,
    pub(super) pattern: Pattern,
    pub(super) bands: usize,
    albedo: [f32; 3],
    pub(super) contrast: f32,
    wear: f32,
    variation: f32,
    grain_scale: f32,
    seed: u32,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum Pattern {
    Plain,
    Transverse,
    Longitudinal,
}

pub(super) fn catalogue() -> &'static [Style; 20] {
    &styles::STYLES
}

/// Chosen once during bring-up and reused by the surrounding grass.
#[derive(Resource, Clone, Copy)]
pub(crate) struct Surface(Option<&'static Style>);

impl Surface {
    /// Zero (old recordings) and unknown IDs retain the original surface.
    pub(super) fn from_id(id: u8) -> Self {
        Self(
            id.checked_sub(1)
                .and_then(|i| catalogue().get(i as usize))
                .filter(|style| style.id == id),
        )
    }

    pub(super) fn mesh(self, upkeep: Upkeep) -> Mesh {
        self.0.map_or_else(
            || Sward::mow(upkeep, Pitch::TURF_TILE),
            |style| style.mesh(upkeep),
        )
    }

    pub(super) fn tile(self) -> f32 {
        Pitch::TURF_TILE * self.0.map_or(1.0, |style| style.grain_scale)
    }

    pub(super) fn colour(self) -> Vec3 {
        self.0.map_or(Vec3::ONE, Style::colour)
    }
}

impl Style {
    pub(super) fn mesh(&self, upkeep: Upkeep) -> Mesh {
        let mut sward = Sward {
            positions: Vec::new(),
            normals: Vec::new(),
            uvs: Vec::new(),
            tints: Vec::new(),
            ground: Vec::new(),
            indices: Vec::new(),
            upkeep,
        };
        let colour = self.colour();
        let lengthwise = self.pattern == Pattern::Longitudinal;
        let span = if lengthwise {
            Vec2::new(Field::WIDTH, Field::LENGTH)
        } else {
            Vec2::new(Field::LENGTH, Field::WIDTH)
        };
        let count = self.bands.max(1);
        let width = span.x / count as f32;
        let offset = Vec2::new(self.seed as f32 * 3.71, self.seed as f32 * 1.93);
        let turned = Vec3::ONE + (upkeep.mow() - Vec3::ONE) * self.contrast;
        let rolls: Vec<f32> = (0..count)
            .map(|band| {
                1.0 + 0.18 * (Sward::hash(Vec2::new(band as f32, self.seed as f32)) * 2.0 - 1.0)
            })
            .collect();
        let tint = |band: usize| {
            colour
                * if band % 2 == 1 {
                    Vec3::ONE + (turned - Vec3::ONE) * rolls[band]
                } else {
                    Vec3::ONE
                }
        };
        // Shared boundaries keep the surface closed. Outermost boundaries stay fixed.
        let drift = |boundary: usize, along: f32| {
            if boundary == 0 || boundary == count {
                return 0.0;
            }
            let phase = boundary as f32 * 2.399_963 + self.seed as f32;
            0.12 * ((along / 21.3 + phase).sin() + 0.4 * (along / 8.7 - phase).cos())
        };
        for band in 0..count {
            let from = -span.x * 0.5 + width * band as f32;
            let rows = (width / Sward::CELL).ceil() as usize + 2;
            let columns = (span.y / Sward::CELL).ceil() as usize;
            let base = sward.positions.len() as u32;
            let pass = Pass {
                tint: tint(band),
                facing: if band % 2 == 1 { -1.0 } else { 1.0 },
                phase: Vec2::new(
                    Sward::hash(Vec2::new(band as f32, self.seed as f32)),
                    Sward::hash(Vec2::new(self.seed as f32, band as f32 + 29.0)),
                ),
                lines: [0, Pitch::STRIPES],
            };
            for row in 0..=rows {
                let fraction = if row == 0 {
                    0.0
                } else if row == rows {
                    1.0
                } else {
                    (Sward::FEATHER
                        + (width - 2.0 * Sward::FEATHER) * (row - 1) as f32 / (rows - 2) as f32)
                        / width
                };
                for column in 0..=columns {
                    let along = -span.y * 0.5 + span.y * column as f32 / columns as f32;
                    let across = from
                        + width * fraction
                        + drift(band, along) * (1.0 - fraction)
                        + drift(band + 1, along) * fraction;
                    // A rotation, not a reflection, preserves triangle winding.
                    let point = if lengthwise {
                        Vec2::new(along, -across)
                    } else {
                        Vec2::new(across, along)
                    };
                    let uv = Sward::grass_uv(
                        Vec2::new(across, along),
                        pass,
                        Pitch::TURF_TILE * self.grain_scale,
                    );
                    let edge_tint = if row == 0 && band > 0 {
                        (tint(band - 1) + pass.tint) * 0.5
                    } else if row == rows && band + 1 < count {
                        (tint(band + 1) + pass.tint) * 0.5
                    } else {
                        pass.tint
                    };
                    sward.positions.push([point.x, 0.0, point.y]);
                    sward.normals.push([0.0, 1.0, 0.0]);
                    sward.uvs.push(uv.to_array());
                    sward.tints.push(edge_tint);
                    // Keep wear at the actual goals; shift only the organic variation.
                    sward.ground.push(
                        Sward::ground(point, true, self.wear + (upkeep.worn() - 1.0), 0.0)
                            * Sward::ground(
                                point + offset,
                                false,
                                0.0,
                                self.variation + (upkeep.rough() - 1.0),
                            ),
                    );
                }
            }
            let stride = (columns + 1) as u32;
            for row in 0..rows as u32 {
                for column in 0..columns as u32 {
                    let a = base + row * stride + column;
                    sward.indices.extend_from_slice(&[
                        a,
                        a + 1,
                        a + stride,
                        a + 1,
                        a + stride + 1,
                        a + stride,
                    ]);
                }
            }
        }
        sward.build()
    }
    // Apply the same palette to the surround; the shared texture still grades upkeep.
    fn colour(&self) -> Vec3 {
        let original = LinearRgba::from(Pitch::MOWN);
        let shade = LinearRgba::from(Color::srgb(self.albedo[0], self.albedo[1], self.albedo[2]));
        Vec3::new(
            shade.red / original.red,
            shade.green / original.green,
            shade.blue / original.blue,
        )
    }
}
